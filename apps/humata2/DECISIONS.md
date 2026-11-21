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
  - (a) Fold each into the nearest row: comments to its own session are a read of its own session (IN-16, no rule); the Global board and other channels share the row "Global board, or other channels and their boards: anything"; restore and rebuild go through the start or fork rows for that machine; archiving another session goes through "Stop or archive another session" (Ask me since D306; deleting another session is its own row, "Delete another session", Never); a read-only watch goes through "Read another session's files".
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
- Launching/Resuming → STARTING, Running → WORKING, Exited and Stopped → EXITED, ResumeFailed → RESUME_FAILED; a session with no agent record → IDLE; an unfinished start shows STARTING.
- Amended 2026-11-17 (owner via Fable): Stopped used to map to IDLE. A stopped agent has no holder, so it is as gone as one that exited. Only EXITED offers Resume and turns off "Stop the agent" (SE-6, SE-14, D671). IDLE is for a running agent between turns (D169). To reverse: map `AgentState::Stopped` back to Idle in `session_status` (`daemon/humatad/src/session_service.rs`).
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
  - **Building (2026-11-18):** the remote machine shell with no session moves from Not wired to building. VERIFY 0.2.3 found it live: `humata shell <remote>` is refused with "can't open shells elsewhere yet" (`terminal.rs`). The host relays OpenMachineShell over the peer link to the machine's own daemon, which opens the shell in its own tab model. Tab and attach are relayed as a remote session's are. The shell then lists on that machine's page and takes Stop and End like any other holder (D784). Its Linux VM-only tests are written now and run in the Linux batch. It isn't on 0.2.4's critical path: if it isn't ready, it ships in the next release.
    - **Done (2026-11-18), daemon and CLI:** the cause was a name, not a missing relay. The host already relayed OpenMachineShell, but it sent the machine its host-side name, and a box knows itself by its own name, so the box refused it as "elsewhere". The host now sends no name (the box's own) and puts its own name for the box on the returned Tab. `AttachOpen.machine` (new field 10) names a machine shell's machine; the host relays such an attach to that machine with the same Global input check and UP-1 gate, and `humata shell` sets it from the Tab. ListRunningHolders, Stop, Resume and End already relayed by machine and tab (D784). An offline box refuses the open with Unavailable (MachineOffline: "use Reconnect now on the machine's page, then try again"); nothing is queued. Tests: `machine_shell_remote_test` (rig), and `machine_shell_remote_linux_test`, built for Linux only for the VM batch.
    - **Open:** the app has no machine-shell tab yet (`clients/terminal_link::open` sends no machine), `humata attach` needs a session so a machine shell can't be re-attached by tab from the CLI, CloseTab and Type don't reach a remote machine shell (End does), and a machine the host doesn't know is reported as offline rather than not found.
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

**Part 1 superseded by D185** (context-only forks share and edit the parent's code); parts 2 and 3 stand, GiveOwnCopy included. **Part 2's byte-prefix copy is superseded by D307** (a conversation adapter writes the kept turns in the CLI's own format, CV-2).

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
- **Open:** a remote machine's agent reaches its own machine's daemon; carrying the normalized card over the host link is for the remote-work lane. Until then a remote machine's hook gets no decision, and the TUI asks. Closed by D744: the machine journals the card to the host.
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
- **Superseded (2026-11-16, D288, PR C5):** every pasted PR now offers Review this PR · owner/name #n at once; it opens the session already reviewing that PR, or else the Review a pull request dialog with the link pasted and This Mac picked.

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
- **Agent continuation (2026-11-16):** explicit per-profile native/CLI opt-in, full structured provider IDs and surviving provider state are required. A holder's confirmed abnormal child exit may launch one fresh terminal using resume-only argv; missing SSH/holder alone never starts another agent. A trusted native orchestrator can attest completed machine loss separately. Reservation+identity persist before spawn; Stop intent prevents recovery and shutdown drains in-flight launch. Foreground conversation forks pin their actual child ID and explicit paths, revoke parent bindings and reject late parent events. The authoritative workspace identity wins both on restart and after a companion settings write failure. Launch receipts are not provider acceptance; unfinished input is not replayed, and background jobs/process memory are not recreated. See `terminal/host/AGENT_RECOVERY.md`; actual Codex, Claude Code and Copilot fork/kill/resume fixtures pass. This extends the experiment, not the main daemon's API.
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

## D225: A machine journals its Inbox and sends the journal to its host (AWAY manager, 2026-11-16). Provisional
- **Problem:** AW-3, AW-5 and AW-6. An agent's request made on a machine while its host is away waits in the machine's Inbox, and the user may answer it there with the passphrase login (D223). Before this, nothing reached the host: the uplink only took policy copies, and nothing wrote the store's journal.
- **Pick:** on a role-machine daemon, every new Inbox request, every answer and every withdrawal is also appended to the machine's journal (store `journal`), whether or not the host is reachable. There is one path, and replay is the same as live. An entry is a `JournalEntry` whose `operation` is `inbox.request`, `inbox.answer` or `inbox.withdrawn`. Its `request` is the `InboxRequest` as it stands after the change, its `principal` is the asking session, and its `policy_revision` is the copy the machine judged by. The machine sends its unacked entries once the host's Resume has said what it already holds (`from.journal_through`), then each new entry as it is made. The host's `UplinkAck.journal_through` ends a resend. An entry is kept until it is acked, across restarts.
- **Open:** the Inbox write and the journal append are not one store transaction, so a crash between them loses the entry; the request itself is kept. Changes the Inbox makes while restoring at start-up are not journaled. The host-side merge into its Inbox (first answer wins, AW-10; Always allow becomes a host grant, AW-12) is T7.
- **Reverse:** stop appending; the uplink ignores Resume's journal_through.

## D226: The host replays a machine's Inbox journal (AWAY manager; picked by a Fable 5.2 high subagent, 2026-11-16). Provisional
- **Problem:** a machine journals every Inbox change and sends it up (D225), but the host ignored `MachineToHost.journal`, always resumed from 0 and never acked. T7 makes the host replay those entries into its Inbox (AW-6, AW-9, AW-10, AW-12).
1. **A replayed request keeps its id and stays held by the machine; the host shows a mirror and relays answers.** Ids are `rq_<uuid4>`, so the same id is safe. The host stamps `InboxRequest.journal = {machine, seq}`, which marks "held on that machine". The card is taken as the machine wrote it, except that the entry's session must be one the host's record places on that machine and must be the card's requester, and the channel comes from the host's record. The mirror is saved with `deferred = false` and no `asked`, so `kept::restore` keeps it. AnswerRequest on the host runs the usual user, presence, still-open and expected_revision checks and checks the answer fits the card, then sends `InboxAnswerRelay` (the host's user as `by`) on that machine's uplink, releases the Inbox lock and waits for the mirror to leave Pending. The machine applies it in its own Inbox as the host's user (first answer wins there, under its one lock) and the result comes back as an `inbox.answer` entry. It returns the request as it then stands when the recorded answer is the one sent, else `already_answered` or `request_withdrawn`. Link down: `machine_offline`, nothing sent, the mirror stays pending; the fix says to reconnect, or answer on the machine with `humata login` and the approval passphrase. No verdict in 10 s, or the link drops mid-wait: `machine_offline`, saying the answer was sent but not confirmed and the card will show the outcome when the machine is back. Cut: queued host answers, `UplinkAck.answers_through`, `LocalAnswer`; Answer all leaves mirrors in `skipped`. Reverse: mint a host id and hold the call on the host.
2. **An `inbox.answer` sets the mirror to the entry's state, and the host writes the credit.** Applying an entry is an upsert by id; a request the host never saw is created already settled. `answered.by` stays the host's own user only when the host has a relay outstanding for that id with that answer; otherwise it is the user via CLI on that machine (AW-6). The same state again is a resend and a no-op; a different one is rejected and acked, and the first recorded stands (AW-10). Approve, deny and reply are recorded with no re-check: they already took effect on the machine. Always allow (AW-12, AW-9): the host grant is made here, for a passphrase answer and a host-relayed one alike, from the card's `always.target` re-resolved against the host's rules as they are now (`check_ask_ahead`), installed the way a restart installs a kept grant, and the mirror is saved with `asked` set to that target. Rule still Ask: the grant is made and its policy copy goes to the machine before the ack. Rule now Allow or already covered: no grant. Rule now Never, target gone, or a host-only request credited to the machine: no host grant; the answer is still recorded and the log says why. On the machine, a grant made there is kept only while its `inbox.answer` entry is unacked; on the ack it is dropped and marked revoked. Cut: AW-9's line in the agent's terminal for a refused grant. Reverse: take the machine's `by` as sent.
3. **An `inbox.withdrawn` sets the mirror to Withdrawn with the machine's reason.** A host answer waiting on it returns `request_withdrawn`. An unknown id is created withdrawn; already withdrawn is a no-op; already answered: the first recorded stands and the entry is rejected. The host's own withdrawals of a mirror send nothing down. Cut: `RequestWithdrawnRelay`.
4. **"Applied through" is one row per machine in the host's store, written in the same transaction as the Inbox row.** Table `machine_journal(machine PRIMARY KEY, applied_through)`; one store call upserts the Inbox record, if any, and sets the mark. Entries are applied in arrival order in the link's loop, taken off before the reports sink (a journal entry must not end the status snapshot). The host's Resume carries the mark; the host acks after each entry is applied or rejected. An entry that can't be decoded or applied (unknown operation, a session not on that machine, a state that disagrees with its operation, a second different answer) is rejected and acked, and the log names machine, seq, operation and reason. Only a failed host store write holds: nothing advances or is acked, later entries on that link wait, and the host re-sends Resume with the mark after 30 s. On the machine a Resume restarts sending from its mark (`sent_through = through`). The mark is dropped when the machine is forgotten or re-enrolled. Cut: the journal epoch; `HelloResponse.journal_high_water` stays 0. Reverse: keep the mark as a settings row.
5. **Board writes stay out of T7.** Nothing journals them yet. T7 replays only `inbox.request`, `inbox.answer` and `inbox.withdrawn`; any other operation is rejected and acked. Board replay (SL-6–SL-8) lands with the board journal and reuses pick 4.
- **Also:** a machine's start-up withdrawals (`kept::restore`) are journaled too, so the host's mirror doesn't stay pending. GRPC.md §9.4 is amended: a pending request stays held by the machine, and local answers go up as journal entries, not `LocalAnswer`.

## D240: A testonly fault build of the daemon traces, arms and power-cuts its fault points (SAFE manager; picked by a Fable 5.2 high subagent, 2026-11-16). Provisional
- A testonly `humatad2_faults` (`admin` + `faults`, DAEMON_DEPS with store/holder/revive/sessions/transfers swapped for `*_faults`, plus `durable_faults`); `humatad2_admin` unchanged. New `daemon/humatad/src/fault.rs`: `point()` delegates to `humata2_store::fault::point`, `hold_point` moves there, the five `stop_point` copies go; under `faults` SIGUSR2 power-cuts the daemon (as the holder does). `HUMATA2_FAULT_TRACE=<file>` appends each point name reached; `HUMATA2_FAULT_ARM=<file>`: hits don't count until the file exists (in `store/src/fault.rs`, so sessions/revive/transfers points trace too). `tests/common` starts the faults build (`HUMATAD2_FAULTS`, `humata2_holder_faults`) whenever a start's env has STOP_AT, HOLD_AT, POWER_CUT_ROOTS or FAULT_TRACE; boxes too. Risk: ~48 humatad `std::fs` writes bypass humata2_durable, so a cut doesn't undo them (T6).

## D241: The restart report names each interrupted operation and how it ended (SAFE manager; picked by a Fable 5.2 high subagent, 2026-11-16). Provisional
- `restart::Operations` recorder in humatad: each start-time recovery records `{id, kind, step, state}` with state `pending{waits_for}`, `finished`, `undone{reason}`, `stuck{reason}`; `pending` when its journal is read, the outcome when it settles; latest line per id wins. Appended and fsynced to `restarts/<time>/operations.jsonl` (torn last line ignored) and logged; buffered until the report folder is known. No revive/restart.json/proto change.

## D242: humata doctor restarts repairs what it can without losing anything; lossy repairs need --allow (SAFE manager; picked by a Fable 5.2 high subagent, 2026-11-16). Provisional
- `humata doctor restarts [report]` (newest by default) reads `restart.json` + `operations.jsonl` with its own serde types. Safe repairs through one new RPC `DaemonService.RepairRestart{session|tab|store}` (re-runs bring-back for one entry under the Starter lock; store: integrity check, healthy refused, damaged → daemon replies and exits so the next start restores). Lossy repairs listed with ids like `start-fresh:<session>`, run only with `--allow <id>`; `--explain` read-only, works daemon-down. Writes `diagnosis.md` in the report folder (`--diagnosis -` takes the skill's text). Skill `skills/restart-doctor/SKILL.md`. `DoctorError` in the CLI.

## D243: The PER-28 power-cut suite runs a host and one box on the fault build against an inventory (SAFE manager; picked by a Fable 5.2 high subagent, 2026-11-16). Provisional
- `tests/power_cut_{host,machine,both}.rs` over `tests/common/power_cut.rs`, in RIG_TESTS (large), sharded. Host + one box on the faults build, POWER_CUT_ROOTS on each side's state and holders. Setup puts every PER-25 item on both sides; each operation (start, local fork, remote fork, move, remote archive+restore, comment send, transfer) runs once traced, then once per first armed hit of each traced name. Daemons die before holders; waits on exits, no sleeps. Check: an Inventory through the public API: untouched items equal, touched items before-or-finished, anything else named in `restart.json` or `operations.jsonl`; comments typed once; terminal output a gap-free prefix.

## D260: "Stop all sessions…" on a machine: the words, where it lives, the CLI and frames 72–76 (STOP manager; picked by a Fable 5.2 high subagent, 2026-11-16). Provisional
- **Words:** the action is **Stop all sessions…**, the state **Sessions stopped**, the way back **Start sessions**. Provider VM power keeps "stopped" / **Start** (MP-21, HO-3, golden 28's cloud-vm-3), so the two never read alike. A provider VM that is off and also Humata-stopped reads as today, plus a page line "Sessions were stopped by you at 14:02 and stay stopped after the VM starts".
- **UI:** the last item, red, after a separator, of the machine page's ⋯ menu and of the sidebar machine row's context menu (frames 59 and 28 unchanged). No Hosts-row Stop, no app-wide stop (follow-up if wanted). **Start sessions** is in the page's Stopped banner, the Hosts row's one action, and the sidebar context menu. The confirm names what ends (agents, shells, 2 s to quit then killed), what is kept, that agents can't type, call Humata or be answered and their open requests end, and that it stays stopped across reboots; for This Mac also that other machines keep running and operations in flight are cut and finished or undone at Start. Buttons Cancel / red **Stop all sessions**; the dialog is the in-app presence (SEC-10). Start has a lighter confirm.
- **CLI:** `humata host stop|start <machine>` from the host (`machine` a visible alias of `host`); `humata stop|start` on a machine means that machine. `humata provider start|stop` unchanged; each help names the other. Stop prints what ended and how to start; Start prints the restart report's lines.
- **Frames** (new `desktop/tests/design/stop.rs`, appended rows only, no existing golden changes): 72 stop-confirm over frame 59; 73 machine-sessions-stopped (banner, grey session rows "Stopped", New session disabled "Start sessions first"); 74 hosts-sessions-stopped (frame 28 with devbox-1 "Sessions stopped" + Start sessions; sidebar square glyph, dimmed sessions); 75 start-confirm; 76 stop-not-delivered (orange banner, Cancel stop).
- **Undo:** rename the strings; drop the menu items, commands and frames 72–76.

## D261: A stop covers one machine, is a file on that machine, and is a restart into a sweep (STOP manager; Fable 5.2 high, 2026-11-16). Provisional
- **Scope:** one machine's own sessions. Stopping This Mac leaves boxes running (AW-1) and the host keeps linking them.
- **Record:** `<data>/stop/this.json` via humata2_durable (temp, fsync, rename, sync_dir): `{stop_id, stopped_at_unix_ms, by {kind user|agent|terminal, via app|cli|ssh|direct, machine, session?}, reason?, ended? {agents, shells, killed, left}}`. A file, not a store row, so a PER-31 restore from an older copy can't bring wild agents back, and it is read before the store opens. The host keeps `<data>/stop/<machine-id>.json` (same shape + `delivered`), removed on remove/forget, learned from the stop's reply and a new `HelloResponse.sessions_stopped`; clients get `Machine.sessions_stopped`, read from the host's copy so it shows while the link is down. No new Reachability value.
- **Daemon stays up by re-exec:** Stop writes the record, replies, re-execs humatad2; a boot with the record skips every start-time recovery and runs the sweep. It serves the user's reads, archive, delete, rename, boards, Settings, Start and the host's links; it refuses every Agent and Script caller (`Refusal::MachineStopped` in `guard::judge`) and launches, resumes, attach and typing.
- **Sweep, in order:** record written and synced → reply → re-exec → SIGTERM each holder found by its pid file (the holder's own SIGHUP, 2 s, SIGKILL; scrollback synced) → withdraw the Inbox (D264) → wait up to 5 s, then SIGKILL the holder and its program group → check no holder pid or program group is alive → write `ended`. Agents stay `Running{holder_pid}` (never `Stopped`) so Start revives them. No Ctrl-C step: a stop is a shutdown. Fault points `stop.record.written`, `stop.exec`, `stop.holders.signalled`, `stop.inbox.withdrawn`, `stop.holders.gone`, `stop.record.settled`; a cut before the record syncs leaves everything running, any later cut reruns the idempotent sweep at the next boot.
- **Known limit:** a process that left its group with `setsid` can't be found on macOS; `ended.left` and the docs say so.
- **Undo:** an in-process stop (needs an operation registry to cancel) and a settings key.

## D262: Stop reaches a box by racing the peer link and a one-off SSH, idempotent by stop_id (STOP manager; Fable 5.2 high, 2026-11-16). Provisional
- Peer RPC `StopSessions{stop_id, by}` with a 3 s deadline races a one-off SSH `script(...)` (new `Purpose::StopSessions`; its own connection, `ControlPath=none`) running `humatad2 stop --id … --by …`. On the box, `humatad2 stop` asks the local socket; with no answer in 3 s it writes the record itself, SIGKILLs the daemon (the service manager restarts it into the sweep) and SIGTERMs the holders. `humata stop` on the box falls back to it.
- Neither reaches: the host keeps its copy with `delivered: false`, shows frame 76, and sends the stop first thing at the next link-up; Cancel stop or Start clears it.
- **Undo:** drop the deferred delivery.

## D263: Who may stop and start: a new Never-by-default capability, user stops start only by the user (STOP manager; Fable 5.2 high, 2026-11-16). Provisional
- `MachineActionKind::{StopSessions, StartSessions}` → new `Capability::MachineSessionsStop`, tier NeverByDefault; IN-17 row "Stop or start all sessions on a machine | Never"; never low-risk while away. Provider Stop stays `MachineProvision`.
- A stop the user made is started only by the user, whatever the rule. The user needs presence for both (app: the confirm; CLI: `humata login`).
- Exception, Stop only: `humata stop` on the machine itself is accepted without a login when the caller carries no agent or script credential and its process is outside every Humata session (peer pid + `getsid`, as `terminal_session.rs` does). Start never has a no-login or daemon-down path.
- An agent's `humata stop` is judged by the rule: Never denied at once (IN-18); Allow stops its box, itself included, recorded as `by.kind = agent`.
- **Risk:** an uncontained agent that drops its credential and leaves its session with `setsid` can stop its machine, never start it; nothing is lost and the record says how.
- **Undo:** require a login for every stop.

## D264: Requests end as "stopped by you", and what a stopped machine still does (STOP manager; Fable 5.2 high, 2026-11-16). Provisional
- `WITHDRAW_REASON_MACHINE_STOPPED = 8`. The sweep withdraws every pending request of the machine's sessions; on a box through the journaled start-up path (D226), so the host's mirror follows by replay; when the host learns a box is stopped it withdraws what it holds for that machine's sessions. The card reads "Stopped by you · devbox-1, 14:02" (or from a terminal on devbox-1, or by <session>'s agent) from the record's `by`. `Event::RequestGone` unchanged. D225's gap (Inbox write and journal append not one transaction) applies.
- **While stopped:** allowed: the page and sessions, conversations, files, diffs, boards, archive, delete, rename, remove/forget the machine. Refused with "Start sessions on devbox-1 first": new session, fork, move to or from it, resume, rebuild, typing, attach, Open a shell; review comments stay queued. Sessions Start would bring back report new `SESSION_STATUS_MACHINE_STOPPED = 12`, sidebar "Stopped", session header "devbox-1's sessions are stopped" + Start sessions.

## D265: Start is the normal bring-back, and its report names the stop; a reboot while stopped writes none (STOP manager; Fable 5.2 high, 2026-11-16). Provisional
- Start renames `this.json` to `this.starting.json` and re-execs; the boot runs the normal `bring_back`, names the stop, then removes the file. `restart.json` gains optional `after_stop {stop_id, stopped_at_unix_ms, stopped_by, reason, started_by, ended}`; `pb::LastStart` gains `after_stop = 7` (`AfterStop`) and `sessions_stopped = 8`. Frame 47's line: "devbox-1 started at 14:40 after your stop".
- A reboot while stopped writes no report (a report would list every session as failed) and sets `LastStart{started_at, sessions_stopped}` so waiters don't hang; PER-29 gets that sentence.
- Fault points `start.record.renamed`, `start.exec`, `start.report.written`, `start.record.removed`: a cut before the rename leaves it stopped; after it, the next boot brings back again and still names the stop. `operations.jsonl` unchanged.
- **Unverified:** the app reconnecting by itself after its daemon re-execs; re-exec from a multi-threaded tokio process (needs a test); the macOS sandbox denying a contained agent signals and writes to `<data>/stop/`; saved shell scrollback readable with no holder.

## D227: AW-7 rows 2, 3 and 6 on a machine: forwarded board calls, journaled board writes, the last copy, and holding for the host (AWAY manager; picked by a Fable 5.2 high subagent, 2026-11-16). Provisional
- **Problem:** on a machine, an agent's board calls read and write the machine's own store whether the host is linked or not (nothing uses `ForwardedCall`; DECISIONS line 1033 lists it as AWAY's debt). So AW-7 row 2 (reads from the last copy), row 3 (allowed writes journaled, SL-5–SL-8) and row 6 (Wait holds calls that need the host) have no host answer to copy and no host write to replay.
0. **A machine forwards its agents' board calls to the host.** One table, `away/registry.rs`, lists Read rows (`BoardService/ListItems`, `GetItem`, `GetItemHistory`), Write rows (`CreateItem`, `UpdateItem`, `MoveItem`, `DeleteItem`, as journal operations `board.item_create`, `board.item_update`, `board.item_move`, `board.item_delete`) and Held rows (row 6). For an agent caller on a machine, `BoardSvc` no longer touches the local store: while reachable it sends `ForwardedCall` (agent principal, method, request bytes, same idempotency key) up and answers with the host's `ForwardedResult`. A drop mid-call is `host_unreachable`, retryable. The host takes `MachineToHost::Call` off before the reports sink, checks the session is one it places on that machine, and runs the existing `BoardSvc` functions as that agent under the host's rules (TP-8); replay uses the same dispatcher. Cut: `ForwardedResult.pending` (an Ask comes back `needs_approval`); `humata` board commands by a machine-logged-in user; topology, view and comment methods; see D268.
1. **Row 6: the hold lives in the handler and is not durable.** The Guard marks a `WaitForHost` refusal with a private status metadata key; `Guard::held(caller, run)` runs the handler body, and on that mark waits for the link, strips the mark and runs it again, so it is judged anew. Skip's refusal has no mark and fails at once. Held: `InboxService/RequestAccess` (the wait is outside the Inbox lock), reads or copies from another machine, start or fork elsewhere, a note elsewhere, push. Bound: the call's `grpc-timeout`, else 40 s for an agent (under MCP's 45 s wait) and none for a user at a terminal. Past the bound: `host_unreachable`, retryable, nothing kept. Cut: surviving a restart or the client leaving; waiting for a fresh copy before judging again. SPEC row 6 away cell: "waits until the host is back, then is judged again and done or asked; a call that outlasts its wait is refused saying the host is away, and can be repeated. Under Skip denied saying the host is away; never low-risk".
2. **Row 3: board writes are journaled; comment and view drafts are refused.** `JournalEntry` unchanged: `operation` as above, `request` the request bytes as received, `principal` the session, `policy_revision` the held copy's, `base_revision` the request's `expected_revision` (unset = later write wins, SL-7). Reply `delivery = JOURNALED` with a `JournalPosition`, sent only after a fallible append commits. Journaled only: creates on its own session board or the channel board, and edit, move or delete of items in `PolicySnapshot.owned_items` owned by the caller (the host fills it; a change bumps the policy revision); any other item is `host_unreachable`. Host replay dispatches by prefix (`inbox.` unchanged, `board.` to board replay): decode and principal checks as D226; re-check as that agent under the host's rules now (AW-9: an Ask or refusal is not applied, kept lost in the item's history plus one line in the agent's terminal); apply per field; the mark moves in the same transaction as the board write. SL-6: a replayed value over a field changed since its base wins and the host's value is kept as a `lost` entry (GRPC §9.5.2). SL-8: the store's existing tombstone rules. Cut: provisional ids (a journaled create has no id until replay); Asks on other sessions' items while away; an away read showing its own journaled writes; comment and view drafts while away. SPEC: AW-7 row 3 drops "comment and view drafts"; a new row "Comment and view drafts | done | refused, saying the host is away; nothing is kept"; SL-5 "(board items, questions to the user)" and "A journaled new item has no id until the host replays it."; SL-6 "the replayed write wins, as the later one to reach the host"; GRPC §9.6 and the MCP provisional-result wording go.
3. **Row 2: the last copy is the forwarded read responses.** A machine store table keyed (session, method, sha256 of the request) holds the response bytes and when the host answered; 64 per session, oldest dropped, none over 256 KiB, rows go with the session, kept across restarts. Away, the Guard judges first (Never isn't served); a hit returns the bytes with response header `humata-stale-as-of` (RFC 3339), which the MCP bridge maps to `stale_as_of`; a miss is `host_unreachable`. `GetChannel` and `ListSessions` answer from the machine's own records, with the header set to when the link dropped. Cut: prefetch on link-up. SPEC row 2 away cell: "answered from the last copy, marked with the time it was made; a read the machine holds no copy of is refused, saying the host is away".
- **Order:** row 6; forwarding; host board replay (before any machine journals a board write, since the host rejects and acks unknown operations and the machine then deletes them); machine board journal; last copy; then the AW-9 terminal line and the approved-Ask slice.
- **Reverse:** drop the registry rows (board calls go back to the local store); drop the cache table; drop the hold helper (the 40 s is one constant).

## D266: AWAY closes with Contain agents enforced; the record field and Info card wording move to POLISH (picked by an Opus 5.5 high subagent while Fable is rate-limited, 2026-11-16). Provisional
- **Problem:** AWAY.md T8 listed four leftovers: `Session.contained`, the contain setting reaching machines, `<session>/.humata/` not git-excluded, and the Info card wording.
- **Pick:** the copy to machines (80afbeae: PolicySnapshot fields 9/10, applied in `policy_copy.rs`) and the git-exclude (ce242d25: `contain/src/git_exclude.rs`, called at every contained launch) are done. `Session.contained` and the Info card wording move to POLISH, V1 at the latest: the record decides nothing (each launch reads the setting again, and a launch that can't be contained stays stopped).
- **Before AWAY closes:** a machine that never received a copy reads Contain as off, so a test must show the copy arrives before the first relayed start.
- **Why not fail open or leak:** the `.humata/` copy can hold secrets (`.claude.json`, `settings.json` env); it sits beside the repo worktrees, not in one, and is excluded besides.
- **Reverse:** pull the two items back into AWAY.

## D267: AW-9: a write refused at replay shows as one line under the agent tab, kept by the host (AWAY manager; picked by an Opus 5.5 high subagent while Fable is rate-limited, 2026-11-16). Provisional

1. Carrier: nothing goes to the machine; no peer.proto change. The host judges at replay (board_service/replay.rs), holds the session record, and every desktop/web viewer reaches the terminal through the host. An ack field would lose the line if the link dropped between mark commit and ack.
   Host-facing proto in api/proto/humata/v1/sessions.proto:
     // AW-9: board writes this session's agent made while the host was away
     // that the host's rules no longer allowed at replay, from the latest
     // link-up that left any out. Unset when none has.
     message AwayWritesLeftOut {
       uint32 count = 1;
       repeated BoardItemId items = 2;      // items whose history keeps them as lost (first few)
       repeated string not_made = 3;        // titles of refused creates (first few); their only trace
       bool refused = 4;                    // true if any now refused (Never); false if all now Ask
       google.protobuf.Timestamp at = 5;
     }
     // in message Session (verify 31 is the last used field):
     AwayWritesLeftOut away_left_out = 32;

2. Display: the UI's existing terminal notice slot. ui/src/terminals.rs Terminals::body(.., notice, ..) draws under the last row, outside the vt grid. ui/src/session_tabs.rs (~L674-705) puts the PER-17 reboot line there for tab 0 from Session.interrupted_at (ui/src/resume_failed.rs reboot_line). New line goes in the same column, tab 0 only. Never written to pty input or output.
   SPEC AW-9 new wording: "A write a tightened rule no longer allows is not applied: an edit is kept in the item's history as lost, a new item is not made, and the agent's terminal shows one line under its output saying how many of its writes weren't applied and why, until a later reconnect replaces it. The line is Humata's, not the agent's: nothing is typed into the agent."

3. Text (UI formats from fields, one sentence; stricter wording wins when mixed):
   refused: "2 board changes this agent made while the host was away weren't applied because the channel's rules now refuse them (kept as lost in PAY-12's history); allow them in Agent access, then ask the agent to make them again."
   all Ask: "1 board change this agent made while the host was away wasn't applied because the channel's rule now says Ask me (kept as lost in PAY-12's history); ask the agent to make it again and you'll be asked."
   creates only: parenthetical becomes "(new item “Fix login” wasn't made)". Several items: "PAY-12 and 2 more".

4. Persistence: host store, written in the SAME transaction as the machine's applied-through mark (edits: Store::replay_board_entry in daemon/store/src/machine_journal.rs; refused creates: the Stopped::Rejected branch's apply_machine_journal(machine, seq, None) needs a variant that also writes the summary). Redelivered seq hits the mark check, so no double count. A write from a new link-up replaces the summary; same link-up adds. peer_client.rs Up records when its link came up and passes it through HostSide::replay. Survives app and host restarts. After writing, publish a session Updated feed event.

5. Tests:
   - replay.rs units (host(), rule(), create(), retitle()): Ask on an edit -> item unchanged, history lost, summary count 1 items [PAY-1] refused false; Never on a create -> no item, not_made has title, refused true; same seq twice -> count unchanged; second link-up batch replaces; NoItem/undecodable/allowed -> no summary.
   - store test in machine_journal.rs: summary + mark land together, survive reopen.
   - integration on tests/common/remote_rig.rs (real host): session on a box, link_lost, agent own-board edit JOURNALED (add an agent-call path with the session token header like away_machine.rs as_agent if missing), host sets OwnBoardWrite Never, link_back, wait_session until away_left_out.count == 1, item revision unchanged + history lost, restart_host -> summary still there, agent tab got no input.
   - clients/live/src/map.rs mapping test; UI headless test: tab 0 shows the line, shell tabs don't.

Record only Ask, Refused, Never rejections; not NoItem, Archived, Unusable, Undecodable.
Risks to log: a viewer attached directly on the machine, or host-relayed `humata attach`, doesn't see the line (follow-up: a TerminalNotice on attach open); the agent isn't told (deliberate, learns on next read); refused creates have no history (line is the only trace); no dismiss (like the reboot line); D226's refused Always-allow grant line can reuse this later as grants_not_kept.

## D268: An agent's board write the rules ask about, from a machine: linked, the host holds the request; away, the machine holds it and journals the approved write (AWAY manager; picked by an Opus 5.5 high subagent while Fable is rate-limited, 2026-11-16). Provisional

### Today
- Linked: a forwarded write runs `BoardSvc::write` on the host (board_service/replay.rs `run_forwarded`). An Ask makes `inbox.defer_board` on the HOST Inbox; the error comes back as ForwardedResult.error kind `approval_pending`; MCP turns it into pending{request_id} and calls WaitForAnswer on the MACHINE daemon, which doesn't hold the id -> not_found. That is the only break.
- Away: journaled.rs refuses AwayCall::Asks as host_unreachable.
- BUG: replay.rs `allowed()` treats any host Checked::Ask as Rejection::Asks, so writes journaled under "Allow low-risk kinds" are always left out at replay and the D267 line wrongly says "rule now says Ask me".

### Picks
1. Linked: the request lives on the host; approval runs the write there (deferred.rs unchanged).
   - When WaitForAnswer or WithdrawRequest names an id the machine Inbox doesn't hold and the uplink is open, the machine forwards it as a unary ForwardedCall (new registry rows /humata.v1.InboxService/WaitForAnswer and /WithdrawRequest; never stored in the last-copy table).
   - The host runs its own wait_for_answer as that agent (`sees` checks the session), skips keepalives, returns the final WaitResponse bytes (approved: WriteItemResponse delivery=APPLIED).
   - The machine sends its own keepalives every 15 s meanwhile; if the agent leaves, the stream drops and `Leaving` sends ForwardedCallCancel.
   - Link drop mid-wait: host_unreachable, retryable; fix says call inbox_wait again with the same request_id; an approval on the host still makes the change (TP-13).
   - The machine keeps an in-memory set of ids that came back approval_pending from the host; while away a wait/withdraw on one is host_unreachable; any other unknown id stays not_found.
   - Not done: ForwardedResult.pending (reserve field 4); approved_request on ForwardedCall.
2. Replay counts low-risk as the rules are now (bug fix, no proto change): in replay.rs `allowed()`, a host Checked::Ask is allowed when the channel's While I'm away rule is AllowLowRisk listing that capability now (`may_be_low_risk`); otherwise left out as Asks.
3. Away (writes the machine may journal: own board, channel-board add, its own items; under Wait): the request lives in the machine Inbox; approval journals the write with approved_request.
   - journaled(): `proved` check before the Ask branch; unproved stays host_unreachable.
   - AwayCall::Asks carries the AccessRequest; handler calls defer_board on the machine Inbox (journaled per D225, mirrored per D226).
   - Card from the request alone: new-item preview for creates; for edits item key + new values, `before` empty (host BD-15 fills it at display time).
   - Run closure (`FnOnce(String)` takes the request id; call sites board_service.rs, channel_service.rs x2, deferred.rs test) appends the board entry with approved_request=<id>, replies JOURNALED with its position, journals even if the link is back. inbox.answer is appended at commit before start(), so the approval has the lower seq.
   - Deny/withdraw journals nothing.
4. Replay of an entry with approved_request: Allow -> apply. Ask -> apply only when the host mirror of that id is held by this machine (journal.machine), made by the entry's session, Approved, and its preview names the same item (or new-item card for a create); else left out as Asks with a log line. Never -> left out Refused (IN-21, deny_if_never). Lookup via a new async Inbox::approved_on(machine, id, session) before `blocking`.
5. Build 1+2 first (no proto field add). 3+4 if lanes allow before AWAY closes; else POLISH, with the away refusal reworded: "...needs the user's approval under this channel's rules, and a board change can only be approved while the Mac is reachable", fix "repeat once the Mac is back, or set the rule to Allow, or list it as low-risk under While I'm away". Proto field 9 only with 3-4.

### Proto
- peer.proto JournalEntry (3-4 only): `// BD-15, AW-9: the request the user approved for this write, held by this machine. The host applies the entry under an Ask rule only when its mirror of that request is Approved, made by this session and about this item. It never overrides Never.` `RequestId approved_request = 9;`
- ForwardedResult: replace `RequestId pending = 4;` with `reserved 4; reserved "pending";` and comment "an Ask on the host comes back as error.kind approval_pending; the request is the host's, and the agent waits on it through forwarded InboxService calls (D268)". Drop ForwardError::Asked and its Outcome::Pending arm in registry.rs.

### Wording
- GRPC §9.3 row "A board write the rules ask about": Reachable "the host's Inbox request; the machine forwards the agent's WaitForAnswer and WithdrawRequest on it to the host; approving runs the write on the host". Away "for a write the machine may journal, per the away rule: Wait makes a request in the machine's Inbox, and approving it (passphrase on the machine, or the host's relay once linked) journals the write with `approved_request`; Skip `user_away`; a kind listed as low-risk is journaled at once; any other board write `host_unreachable`".
- GRPC §9.4 bullet: the approved write is journaled after its inbox.answer, so the host has the approval before it replays the write.
- GRPC §9.5 step 1: "Under an Ask rule an entry is applied when its `approved_request` names a request the host's mirror shows Approved, held by that machine, made by that session about that item, or when the channel's While I'm away rule lists its kind as low-risk now. Never rejects it whatever was approved."
- SPEC AW-7 row: Reachable "Inbox request; the call waits; approving makes the change". Away "per the While I'm away rule for writes the machine may journal (own board, channel board add, its own items): under Wait an Inbox request on the machine, and the approved write is journaled with its approval; any other board write is refused, saying the host is away (D268)".
- SPEC AW-9 add: "A write the user approved on the machine is applied while the rule still says Ask, and a kind the While I'm away rule still lists as low-risk is applied; Never still leaves it out."
- D227 cut line: replace "a board write approved on the machine ... fails clearly until that slice" with "see D268".
- daemon/mcp/src/outcome.rs (3-4): "journaled" -> "written to this machine's journal and applied when the host replays it".

### Tests
- tests/away_forward.rs (common/away_machine.rs plays host): forwarded update answered approval_pending -> agent's WaitForAnswer on the machine goes up as ForwardedCall for that id -> fake host answers Approved with WriteItemResponse -> wait returns it; link drop mid-wait -> host_unreachable retryable; agent drops wait -> ForwardedCallCancel; WithdrawRequest forwarded; away: host-held id -> host_unreachable, unknown -> not_found.
- replay.rs units: run_forwarded WaitForAnswer from another session -> not_found; Ask+low-risk listed -> applied; kind removed -> Asks; Ask+approved matching mirror -> applied; mirror pending/denied/other session/other item/other machine -> Asks; Never+approved -> Refused; Allow+approved -> applied; same seq twice -> once.
- tests/away_board_journal.rs (3-4): Ask+Wait move of an owned item while away -> approval_pending with machine id, card has BoardChange preview; approve with passphrase login -> board entry carries approved_request after the inbox.answer entry; deny -> nothing; Skip -> user_away; not owned -> host_unreachable, no request.
- remote_rig.rs: linked, ChannelBoardEditOthers default Ask, box agent edits another session's item -> host Inbox request -> approve on host -> wait returns APPLIED, revision +1; deny variant unchanged. (3-4) link_lost -> approve on box -> link_back -> item changed, mirror Approved credited to box user, away_left_out unset; variant approving the mirror from the host after link_back.

### Risks (accepted)
Host-held request can't be answered on the machine; withdraw while away can't reach a host-held card; inbox_list on the machine doesn't show host-held requests; away-approved write replies JOURNALED even if linked again; restart between approval and append reruns; away edit card lacks title/before; failed inbox.answer append leaves the write out as Asks; host trusts the machine's approval on item, not field values.

## D269: A stop's sweep hangs up on the programs itself before it ends the holders (STOP manager; picked by an Opus 5.5 high subagent while Fable is rate-limited, 2026-11-16). Provisional

The stop sweep SIGHUPs each holder's program groups, waits 2 s, and SIGKILLs the groups still alive, counting that holder Killed. Only then does it SIGTERM the holders, wait 2 s, and SIGKILL any holder still alive. So `killed` always means the sweep's own SIGKILL and never races the holder's own 2 s hangup timer. A holder keeps what its terminal printed when its program exits and when it gets SIGTERM, so nothing saved is lost. The sweep counts a pid as a holder under either build's name (`humata2_holder`, `humata2_holder_faults`) or the configured holder's name, so a daemon of one build sweeps the holders a daemon of the other left. New fault point `stop.programs.hungup`, before `stop.holders.signalled`. Start also waits up to 5 s on the state lock, a blocking lock on a thread of its own, not a retry loop. A stop or start lets go of the lock before it re-execs, because under load an old daemon's lock can linger for a moment after it is gone.

## D270: A leaving daemon ends idle response streams at once (STOP manager pick, 2026-11-16). Provisional

When humatad2 leaves (a stop, a start, or SIGTERM), every response stream with nothing ready ends at once with UNAVAILABLE, so its client connects again; replies already ready still go out. Graceful shutdown no longer waits 3 s for attached terminals, hooks or Inbox waits. The 3 s grace is kept only for a call still being answered.

## D271: Terminals in the app run on C1's reusable terminal, with humatad as the one owner (TERM manager; picks by an Opus 5.5 high subagent while Fable is rate-limited, 2026-11-16). Provisional

1. The authoritative terminal screen lives in humatad, in a new crate `daemon/vt_authority` on `ghostty_vt`, one per running tab; C1's `terminal_host` is neither embedded nor run beside it. humatad stays the only owner of holders, the store, revive, FORK, SAFE and STOP.
2. The wire stays gRPC Attach with optional fields (checkpoint format, Control, Resized, Detached, ResyncRequired, InputWritten). An attach with no checkpoint format resyncs with input off; raw bytes are never replayed into a replica.
3. D14 holds: several writers, every attach that isn't read-only is a controller, and only the focused, visible replica asks for a resize.
4. A replica's UUID is UUIDv5(tab id, holder generation). The new `TabRecord.holder_generation` is bumped by revive and by a FORK move.
5. daemon/revive stays the only recovery authority.
6. Terminal bytes leave the TCA store, replaced by a per-tab frame link through `clients/terminal_link`, which builds for wasm.
7. TERM covers the desktop; the browser follows in ANYHOST on the same crate.
8. Memory saver is on by default for a tab not visible in any pane for 30 s, with a Settings switch; it never parks a tab with unsent or uncertain input.
9. Checkpoints are not saved to disk at first: after a daemon restart the screen is rebuilt from the holder's kept bytes, or ResyncRequired is sent.
10. `humata attach` filters terminal query replies out of raw input, so a query is never answered twice.


## D272: The daemon lists the repos gh sees from `user/repos`, at most 1000 (NEWCH manager; picked by an Opus 5.5 high subagent while Fable is rate-limited, 2026-11-16). Provisional

SearchGitHubRepos runs the host's `gh api --paginate "user/repos?per_page=100&sort=pushed&affiliation=owner,collaborator,organization_member"`, with `--jq` cutting each repo to the proto's fields (full name, description, private, default branch, pushed at). It keeps at most 1000 repos, most recently pushed first; past that the person types to narrow. gh keeps its own login: Humata never reads, stores or sends the token.

## D273: The New channel sheet asks once and filters locally (NEWCH manager pick, 2026-11-16). Provisional

The app asks for every repo (an empty query) when the sheet opens and filters as the person types. The daemon still honours the query and paging, for the CLI and MCP.

## D274: Repo search is a two-tier fuzzy match (NEWCH manager pick, 2026-11-16). Provisional

A repo matches when the query's letters appear in order in `owner/name`, ignoring case. Repos where the query appears as typed come first, then repos where its letters are scattered; within each tier gh's pushed order is kept. The app and the daemon use the same rule.

## D275: The sheet shows the first 50 matches, then "and N more" (NEWCH manager pick, 2026-11-16). Provisional

The repo list scrolls with about 8 rows visible and holds the first 50 matches, then a faint "and N more; type to narrow".

## D276: This Mac starts ticked (NEWCH manager pick, 2026-11-16). Provisional

New channel ticks This Mac when it is online, otherwise the first online machine, otherwise none. Create with no machine ticked says to pick one. A machine that went offline after it was ticked can still be unticked.

## D277: Ticked repos stay first (NEWCH manager pick, 2026-11-16). Provisional

Ticked repos always show at the top in the order they were ticked, and stay ticked whether or not they match the search.

## D278: Listing gh's repos is a global read (NEWCH manager pick, 2026-11-16). Provisional

SearchGitHubRepos refuses peers. Other callers pass the GlobalRead guard as ListChannels does; an Ask waits in the Inbox.

## D279: The sheet says what gh is doing and how to fix it (NEWCH manager pick, 2026-11-16). Provisional

While the list loads the sheet says "Listing repos from gh…". A failure shows why with its fix (install the GitHub CLI, or run `gh auth login` in a terminal) and a Retry. Create works with zero repos, so a failed listing never blocks making a channel.

## D280: The daemon keeps gh's repo list for 60 seconds, in memory only (NEWCH manager pick, 2026-11-16). Provisional

A successful listing is kept in memory for 60 s and never written to disk. Failures are not kept, so a Retry after a failure runs gh again.

## D281: Every needed update is a VIP "Migrate <thing>" Inbox item (Provisional, 2026-11-16)
The owner: "We should force upgrade daemons and stuff when we detect them" and "our inbox should get vip messages that say 'migrate x'". Picks by the UPGRADE manager with an Opus 5.5 high advisor (Fable out until 2026-11-22):
- a) The version is the bundle id (sha256 of the bundle manifest), reported only when the daemon's binary matches its platform entry; the CLI sends a `humata-bundle` header. Shown as "2.0.5 (a1b2c3d)".
- b) Each mismatch posts a VIP MIGRATE Inbox item held by the host daemon. The app holds the item itself for an older local daemon that can't hold it.
- c) Proto: request kind 8, preview 14, Approve shown as Migrate, success APPROVED, withdraw reason RESOLVED = 9.
- d) "Force" means the item can't be dismissed and everything else is refused. Migrate is still the user's click.
- e) Error `migration_needed` (field 60). A machine at another bundle links read-only plus the upgrade call. A CLI at another bundle is refused everything but ServerInfo.
- f) Files land verified as `.<name>.next`; an `Upgrade` call swaps and `exec_at`s the installed path (STOP's exec). `.staged` is retired. A one-time `launchctl kickstart -k` or SSH restart handles daemons too old to have the call.
- g) An upgrade record with fault points `upgrade.files.staged` … `upgrade.record.settled`, and a store copy first when the schema changes. If the new build fails to reach ready twice, the old build and the copy are put back.
- h) Schema folds into the daemon item; Humata never downgrades.
- i) Each session records its holder build; "Migrate holders" restarts agent sessions at their next idle moment; shells keep the old holder until closed. The holder protocol stays VERSION 1, additive only.
- j) Schema, CLI and host tools found together with the daemon are lines of the daemon's item, and get their own item only when found alone.
- k) Words: "Upgraded to 2.0.5 · 2 min ago", "Read-only until migrated · Open message"; frames 77–83 in `desktop/tests/design/upgrade.rs`.
SPEC: UP-1 added; TS-0, TS-1 and PER-6 reworded.


## D282: How a host stops and starts a box, and who may (Provisional, 2026-11-16)
STOP T4/T5 picks by the STOP manager:
- a) A host stops a box by forwarding `DaemonService.StopSessions`/`StartSessions` over the existing relay link (relay LINK_OWN gains the two paths), not a new PeerService call.
- b) The one-off SSH `humatad2 stop` is a hedge: it starts when the link fails or after 1 s with no answer, so a healthy link never also SIGKILLs a re-executing daemon. A box whose stop has settled ignores a late SSH stop.
- c) Start waits up to 20 s for the box's link to come back.
- d) A box remembers the last stop it applied (`stop/last.json`); a repeat of that id is refused FailedPrecondition, which the host counts as delivered, and `humatad2 stop` exits 0 on it.
- e) The host keeps each box's stop at `<state>/stop/machines/<id>.json` and reconciles it at link-up through `HelloResponse.sessions_stopped`.
- f) Agents may stop or start only through `CAPABILITY_MACHINE_SESSIONS_STOP` (44, never by default); an agent or script can never start a stop the user made (UserOnly); a person typing `humata stop` outside every session is admitted as Terminal and recorded via CLI.

## D283: PR review: Undo strip at the chat tab's foot (Provisional, 2026-11-16)
Each agent view change's Undo sits in a strip at the foot of the chat tab, newest first, 3 shown plus "+N"; changes caused by the code moving show without Undo; Back sits in the Diff header. Redraws golden 68's strip area (advisor sign-off; the owner sees before/after).

## D284: PR drafts share the one comment ledger (Provisional, 2026-11-16)
PR drafts are comments in the same ledger, moving draft → posted with a link; "Make it a draft" converts a note in place (no chat Undo).

## D285: review_sessions store record (Provisional, 2026-11-16)
A `review_sessions` record holds each review session's PRs, its push-allowed switch and agent switches; identity derives the review kind from it on every lookup. It is store migration 23 (TERM holds 21 and 22) and gets a "Migrate store" Inbox item per D281.

## D286: PR polling is the daemon's (Provisional, 2026-11-16)
Every minute while the app reports a review open, else every 5 minutes; one batched `gh` GraphQL query per tick; changes go out on the event feed. Exit flow runs on This Mac; devbox-1 only if the rig needs no network, else a logged gap.

## D287: Submitting a PR review safely (Provisional, 2026-11-16)
Each draft's line is checked locally against the PR's diff first; drafts GitHub would reject are listed with Re-attach or Drop. A review is tied to the clone's commit and recorded before posting; after a restart it is matched against GitHub's reviews, else shown Unconfirmed with Open on GitHub and Post again.

## D288: Ask, agent view pacing, and Review this PR at once (Provisional, 2026-11-16)
Ask is typed into the chat through the existing path, sent once, never stored as a comment. Agent view changes wait 1000 ms after the user's last scroll, key or selection; focus changes collapse to the latest. Reverses D197: Review this PR shows at once from the parsed URL and the dialog says "Fetching … with This Mac's gh…" while it resolves, or the reason and Try again.
- Added 2026-11-16 by main: an agent's `pr_review_start` that passed its Ask may resolve the PRs it names (a read; no GitHub state changes). Without that approval, resolve stays user-only.
- Added 2026-11-16 by main: SetPushAllowed publishes the Session event, because the app is pushed changes and never polls.

## D289: Per-repo diff targets (Provisional, 2026-11-16)
VIEWS picks (Opus 5.5 high advisor):
- a) Each repo's base is translated exactly as the daemon does it; "Comparing per repo" shows only when the repos' specs differ.
- b) A repo left out of a spec gets the default base; a repo outside the session, or named twice, is refused.
- c) ListBases reads the clone's remote branches without fetching.
- d) The default preset uses the repo's real default branch (origin/HEAD), not always origin/main.
- e) `PushChoice.base_branch` becomes `gh pr create --base`; a PR already open against another base is not retargeted; PlanPush takes an optional spec.
- f) Each repo's pick lives in app memory for now. Open gap: it isn't kept across restarts, and persistence is core (to close within VIEWS).
- g) Offered views are kept on the host (`view.offered` in the machine journal); drafts stay on the machine and move with the session.
- h) App views are keyed by id and arrive on the event feed; the Revert snapshot includes each repo's base.
- i) A prepared review opens in the Diff panel; a repo no longer in the session shows "not in this session".
- j) VIEWS owns the diff spec model, pickers, ListBases, the get_diff/get_file_diff fixes, base_branch, review_prepare outside the review chat, the PrepareView dispatcher and /render-humata-review. PR owns PullRequestReviewService, the review chat applying a plan (D80), PR views (VW-13) and /order-humata-review. PR rebases on VIEWS' first commit (review.proto).

## D290: UPGRADE build identity and refusals (Provisional, 2026-11-16; D281 addendum l–o)
- l) A daemon with no `bin/bundle.json` has nothing to compare against: it posts no Migrate items and refuses nothing. A release always ships bundle.json, so this is not a mixed-version fallback; a host with an id still treats a box with no id as mismatched.
- m) A caller counts as a CLI only when it carries a CLI credential (session, script or login token) or the `humata-bundle` header; the app and admin calls aren't held to the release.
- n) bundle.json is read once at daemon start, from `<parent of --state>/bin/bundle.json`.
- o) `migration_needed` (error kind 60) maps to FailedPrecondition; the CLI exits with the PRECONDITION code.
- Wire numbers: `HelloResponse.build = 12` (11 is STOP's `sessions_stopped`), `ServerInfo.build = 10`, `HelloRequest.host_build = 7`, `REQUEST_KIND_MIGRATE = 8`, `WITHDRAW_REASON_RESOLVED = 9`, `RequestPreview.migrate = 14`; BuildInfo/BuildFile/MigrateThing in a new build.proto.
- Frames 77–83 (advisor-signed): one verb, "Migrated to 2.0.5 (a1b2c3d)"; the banner link reads "Open in Inbox"; a failure says "failed to start twice" and warns that migrating again without a fix fails the same way; the holders item ends when the last agent session has moved (open shells are listed but don't hold it); Allow all/Deny all skip Migrate items; a Migrate card omits IN-9 parts 5, 7, 8; 10 goldens (79–81 each card + machine page).

## D291: Per-repo specs on the wire (Provisional, 2026-11-16; D289 addendum)
- ListBases ships with its daemon implementation (VIEWS B3), not in the first model commit.
- The client sends every repo in a per-repo spec: its own pick, else the header preset translated for it (OriginMain → default branch, UncommittedOnly → uncommitted, SinceLastReviewed → last reviewed, SinceFork → fork point). Picking a repo while another session's comparison is shown resets the header to the default branch.
- "Default branch" is the merge base with `origin/HEAD`; "Uncommitted" is HEAD plus everything on disk.

## D292: A holder that doesn't answer never hangs a close (Provisional, 2026-11-16; TERM)
- `humata2_holder::shutdown` bounds its connect and Welcome wait with a named constant (about 3 s). On timeout it returns its own error.
- When shutdown fails, desk close ends the holder by its saved pid: SIGTERM, a short grace, then SIGKILL, using the STOP sweep's helper. The close never waits without a bound while it holds the session lock.
- TERM batch 3: the reply record lives in four columns on `tabs` (migration 22). An answer's record is saved before the answer is sent, so a crash leaves it pending, and the next start marks input uncertain instead of answering again. `terminal_uuid` uses SHA-256 with v8 bits until `uuid` gets its v5 feature.

## D293: Default branch and saved diff targets (Provisional, 2026-11-16; VIEWS wave 2)
- D198's rule "no origin/main means refuse" becomes "no default branch means refuse". The default branch is origin/HEAD, else origin/main. The refusal tells the person to run `git remote set-head origin --auto`. Uncommitted still works without a default branch.
- Each repo's picked diff base is saved with the review state and read back at launch (D289 f). Any session-wide header preset that isn't the default is saved too.
- Push & PR opens each repo's PR against that repo's picked branch, after checking it exists in every clone. An open PR is never retargeted; it gets a note instead. A multi-repo push reports every repo's PR and note (VIEWS F3).
- Gap: marking only one repo reviewed waits until the owner asks for it.
- TERM ui (task 5): the app's replicas use `FitPolicy::Controlling`, not D271's Focused. Headless golden windows are never activated, so Focused would never send the first fit. One desktop view per tab makes Controlling safe. Revisit in ANYHOST, when a browser and the desktop can both control the same tab.
- The ui replaces a tab's replica when its link ends. The live link ends its frames when the holder's terminal UUID changes, so a new holder always gets a new replica (C1 answer 4).

## D294: UPGRADE batch 2 picks p–t (Provisional, 2026-11-16; D290 addendum)
- p: Hello's build travels in `BeginRelayResponse.build = 2`. The link has no credential to call GetServerInfo on its own daemon.
- q: Migrate items live in memory until the store can hold an Inbox item with no session. The host detects again at every link-up, so none is lost. The store change, with D281 d's unique index, is the next UPGRADE task.
- r: Approve answers "not built yet" until running a migration exists.
- s: Detecting the same mismatch again with an unchanged card doesn't bump the revision.
- t: For now only starting a session, typing and writable attach check the machine's release. Writes, moves and pasting images come later.
- `humata-bundle` is stripped from relayed calls. The MCP server sends its own bundle id.

## D295: The header's ⋯ session menu and Rename dialog (Provisional, 2026-11-16; SE-7, FC-17)
- No frame draws the ⋯ menu or a Rename dialog. The menu is the machine page's menu card under the ⋯ button; Rename is a Move-style card with the title field and a "Rename the branch too" box.
- Items, in order: Rename…, Move to another machine…, Stop the agent, Push and open a pull request, Archive session….
- On a Desk session Move is hidden and Archive reads Close session, as the Desk closes rather than archives (DK-11).
- Stop the agent is off once the agent exited. It ends the agent only; the session, terminals and clones stay.
- Push and open a pull request opens the Changes panel, where pushing and opening the pull request already live.
- Renaming the branch is opt-in. The daemon refuses a branch that is already pushed and the dialog shows why.

## D298: Check a PR for new commits now (Provisional, 2026-11-16)
PR owner ask F1, picks by an Opus 5.5 high advisor:
- Refresh reuses CheckHeads; there is no new rpc.
- A manual check publishes `PullRequestHeadsChanged` like a poll and restarts that session's D286 timer.
- A check never moves the clone or tells the agent. Only Update the clone does that.
- A check that finds nothing answers with empty changes plus `checked_at`.
- A check while one is running joins it. A check within 15 s of the last returns the last answer without calling gh.
- `HeadChange` gains `base_branch = 5` and `stack = 6`, so a retarget shows as a head change.
- UI: a "Check for new commits" item on the PR chip menu (the D183 pattern) and a palette command. The notice slot reads "No new commits · checked HH:MM". No frame is needed.

## D299: Reviewed commits per PR (Provisional, 2026-11-16)
PR owner ask F2:
- Marks are keyed by PR (owner, repo, number), not by session.
- A mark records sha, `git patch-id --stable`, reviewed_at, the head at the time, and who (user plus gh login).
- History is an append-only list of entries (MARKED, UNMARKED, CARRIED, WHOLE_PR), each with its commits, head and optional snapshot n.
- The store is the source of truth: tables `pr_commit_marks` and `pr_review_history`, in the PR store migration (23 while it is unshipped, else 24 with its D281 Migrate item).
- RV-6 Mark reviewed in a PR session records WHOLE_PR and pins its `refs/humata/reviewed` snapshot. Marking single commits takes no snapshot.
- After a force push, a commit with the same patch-id is CARRIED. A changed patch is unreviewed again, and merge commits never carry.
- The agent reads marks through a read-only `review_commits` MCP tool. Only the user marks.
- The rpcs are `ListPullRequestCommits` and `MarkCommitsReviewed`.
- The UI is a "Commits · N of M reviewed" disclosure atop the Diff file list, with history under it. It needs a new frame, which goes through the goldens flow.

## D300: Stacked PRs (Provisional, 2026-11-16)
PR owner ask F3. GitHub stacked PRs went GA on 2026-10-06, on github.com only: the REST `/repos/{o}/{r}/stacks` API, and a `stack` object on each PR. `gh stack` is an extension that needs gh ≥ 2.90.
- **Detection:** the native stacks API first, ordered by the base chain. Otherwise one `gh pr list --state open` call, with the base chain worked out locally. No trailer or body-marker parsing.
- **Shared model:** the stack model is a pure crate `//tools/stack/graph`, owned by STACK. Its `Stack{repo, trunk, source, github_number, entries}` is bottom first, with `parent_of` and `children_of`; a base chain may be a tree.
- **STACK agrees (to confirm with that lane):**
  - the base chain is the truth;
  - linear chains are linked natively where available;
  - head branch names stay stable across amends;
  - the crate has no I/O.
- **Diffs:** a stacked PR's diff base is its parent's head, the same base for the D287 check.
- **Navigation:** one review session holds the stack, one clone per repo. A user-only `SwitchPullRequest` adds the PR to the binding, switches the clone and tells the agent. It is refused, with a fix, over uncommitted changes.
- **UI:** a stack strip in the PR header, a picker for two children, and palette Up/Down the stack on ctrl-shift-up/down (not Shift+J/K, D55.9). This is a new frame through the goldens flow.
- **Submit:** posts each PR at its own recorded head.

## D301: A running machine is upgraded only by Migrate (Provisional, 2026-11-16; D281 addendum)
- A machine that already runs humatad2, This Mac included, is no longer upgraded when it reconnects or when the app launches.
- The app's release is staged beside the installed one, as `.<name>.next` files plus `.bundle.json.next` (the bundle last), and the old release keeps running.
- Migrate checks the staged files against their sha256s and swaps them in, the bundle last. It keeps the old daemon as `humatad2.previous` and restarts on the new one.
- A first install still installs the tools and `bundle.json` at once.

## D302: Approve returns at once; arrival ends the item (Provisional, 2026-11-16; D281 addendum)
- Approve on a Migrate item answers at once, with the item Migrating. There is no 60 s wait inside Approve.
- The item ends as "Migrated to <version>" when the machine is next seen at the new bundle: the local daemon restarted on it, or a machine's Hello names it.
- An upgrade that fails leaves the item pending with what failed and how to fix it, so a second click retries.

## D303: Withdrawn Migrate rows are kept (Provisional, 2026-11-16; D281 addendum)
- A Migrate item that ends keeps its row in `inbox_requests`; only a pending one blocks a new item for the same thing and machine, through the partial unique index.
- Nothing prunes the withdrawn rows for now. They are small, one per upgrade. Pruning them comes with the Inbox's own history pruning, if that is ever needed.

## D304: Agents propose a repo for a channel (Provisional, 2026-11-16; ROAM ask 1, SPEC CH-26, IN-17)
- **Problem:** IN-17 made "Change channel settings or repos" Never, so an agent that needs a repo can't even ask.
- **Pick (Opus advisor):**
  - A new row, "Add a repo to a channel", Ask me, placed after "Create a channel". The old row becomes "Change channel settings, or remove a repo from a channel", Never.
  - The target channel's rule decides. A channel agent may propose only for its own channel; any other channel is Never. A Desk agent may propose for any channel.
  - No **Always allow …**: a grant would cover one repo and be spent by its use (IN-14). Allow is offered, because CH-22 undoes it.
  - Agents propose saved repositories or GitHub URLs, never a local path on the host.
  - Adding it to the proposing session too is a second step under "Change its own session": a tick on the card (on by default), or its own ask under Allow (`also_to_session` on `channel_add_repo`).
  - A card whose repo was added meanwhile resolves "Already added". A call for a repo already in the channel is answered at once, "Already in the channel", with no card.
  - A channel Desk agent counts as a channel agent for its own channel only, and its card has no Also add tick (it has no clones).
- **Review:** an Opus 5.5 xhigh judge reviewed these picks on 2026-11-16; its changes are applied above and in CH-26. Still provisional.
- **Affects:** access capability list (`ChannelRepoAdd`, split from `ChannelChange`), MCP `channel_add_repo` (exists), Inbox card kind, frames.
- **Reverse:** make the row Never again; the capability stays split.

## D305: Moving a Desk session into a channel (Provisional, 2026-11-16; ROAM ask 2, SPEC DK-15–DK-19)
- **Pick (Opus advisor):**
  - Ask me or Never only, with no Allow, no Always allow, and no While-I'm-away approval, as the network rule (D20). The user can also move the session directly from the Desk session's menu.
  - Repos the channel lacks are added to the channel on approval, with no opt-out. A clone that belongs only to the session would break CH-9, CH-20 and CH-22.
  - Each clone gets an FC-11 branch at its current HEAD. The card names the starting point when it isn't the default branch, and the dialog lets the user edit the branch name.
  - The move always copies, verifies and then trashes the old folder (MV-14), even on the same machine, so there is one path and the restart doctor can finish or undo it.
  - Afterwards the session is top-level in the channel tree. The Desk switches and standing grants end, its other pending requests are withdrawn and the agent is told to ask again, and the board items it added stay its own.
  - A plain-terminal Desk session is moved by the user only. A closed session must be reopened first.
- **Judge's changes (Opus 5.5 xhigh, 2026-11-16):**
  - The request is checked against the target channel's rule. If that channel's "Add a repo to a channel" is Never, an agent's proposal that would add repos is refused at once; the user's own Move into a channel… still adds them.
  - A same-volume copy may be copy-on-write; too little disk fails before anything changes.
  - A checkout in the scratch or picked folder (the picked folder itself included) with a GitHub remote is offered as a repo to add, ticked by default; one with no remote comes as plain files. A clone's local default branch with unpushed commits is reset to the remote's after the move, since that work is on the FC-11 branch.
  - DK-18 is the named exception to MV-5 (requests) and IN-22 (grants).
  - Answering checks a digest of what the card shows; on a mismatch nothing moves and the card is redrawn (NV-27). One pending move request per session; a new one withdraws the old.
  - The picks stay provisional.
- **Reverse:** offer Allow; keep session-only repos (needs CH-9/CH-22 changes).

## D306: A channel's Desk (Provisional, 2026-11-16; ROAM ask 3, SPEC CD-1–CD-6, CH-11, CH-24, IN-17)
- **Pick (Opus advisor):**
  - The target is the channel or one of its sessions, and a session target includes its forks, later ones too. The scratch folder is `<root>/<channel>/desk/<name>`, beside `main/`, `sessions/` and `shared/`, so no session folder name needs reserving. There are no clones; the agent reads the main clones and `shared/`. It doesn't move to another machine.
  - It uses the channel's rules, and the channel counts as its own. Management switches are grants applied after Never (as D182):
    - on by default: Read its files, and Send notes and questions;
    - off by default: Type into its terminal (typing can answer the target CLI's own permission prompts), Start and fork (with a session target, only forking it and its forks), Stop and archive, and others' board items;
    - never delete. "Any session of the channel" excludes channel Desk sessions, and no switch answers Inbox requests (IN-23).
  - **Default change:** IN-17's "Stop or delete any session but itself: Never" is split into "Stop or archive another session: Ask me" and "Delete another session: Never". D182 checks Never before grants, so under the old row the Stop and archive switch could never apply. D21's fold of archiving another session follows the new Ask me row.
  - Its Inbox requests show in its channel's Inbox, not the global Desk group.
  - A switch blocked by Never says so beside it.
  - The UI is a **Desk · n** tab after Sessions, which is always there. The sidebar Desk row under a channel shows only while one is open.
  - Short-lived:
    - Close sends the scratch folder to the trash at once, with no 7 days and no Keep. Record and conversation stay 30 days via the trash, and the channel's retention doesn't apply.
    - Close always confirms, in one line, that the folder goes to the trash now, even with no Markdown (on the Desk the same label keeps the folder 7 days). The confirmation lists Markdown files and offers Copy to shared/.
    - It closes itself when its target is deleted or archived (after MV-15's undo window) or its channel is deleted. When the target goes and its folder holds Markdown, it stops and shows "Target gone" with Close instead.
    - Restoring one from the trash reopens it and resumes its conversation; refused while its target or channel is gone.
  - CH-24 lists the channel's Desk sessions and closes them as CD-4. Restoring the channel leaves them in the trash; each is restored on its own while its 30 days last, never before its channel.
  - Turning one into a channel session isn't offered; deferred.
- **Review:** an Opus 5.5 xhigh judge reviewed these picks on 2026-11-16; its changes are applied above and in CD-1–CD-5, CH-24 and IN-17. Still provisional.
- **Reverse:** each switch default is one line; a 7-day Keep like DK-11 could replace the instant trash; "Stop or archive another session" can go back to Never, and the Stop and archive switch then shows "Blocked by the channel rule: Never".

## D307: Conversation adapters per agent (Provisional, 2026-11-16; ROAM ask 4, SPEC CV-1–CV-8, FK-3, MV-13, q11)
- **Problem:** fork at a turn and carry work only for Claude's JSONL (D181: byte prefix; carry rewrites the folder path in any text file). The fixtures prove that Codex and Copilot fork and resume natively by exact id, but no fixture reads, truncates or writes a Codex rollout, its SQLite state, or a Copilot session-state folder.
- **Pick (Opus advisor):**
  - One adapter per format (Claude JSONL, Codex rollout + SQLite, Copilot session-state) behind one trait. It lists turns, forks before turn n into a new id without touching the parent, packs and places the files for a carry with field-aware path rewrites, and checks the version.
  - Unknown format: no turns, and a move refuses (MV-3) with Start fresh with a summary, written by the running agent before it stops or else from its scrollback. There is no byte-copy fallback, because a blind rewrite can corrupt a conversation silently. With no adapter or an unknown format, fork from now uses the CLI's own fork by exact id where the fixtures prove it (Codex and Copilot today), otherwise FK-6.
  - Codex and Copilot fork-at-turn stay empty until real-CLI fixtures first pass for their adapter (CV-7). After that, the format check decides: a newer version is known when the strict parse accepts every record, and an unknown record kind or path field makes it unknown; the launch version is recorded. Fixtures run on each new CLI release and a failing release is blocked.
  - Turns are those on the current path; prompts abandoned by /rewind or replaced by an edit are not listed or carried. A fork reads only complete records, so from now during a turn means after the last finished turn. A fork is done only once it has resumed by id. This supersedes D181 part 2's byte-prefix copy: the adapter writes the kept turns in the CLI's own format.
  - Humata never writes a CLI's index (such as Codex's SQLite). If the CLI doesn't find the placed conversation when resumed by id, the move fails at the resume check.
  - Folder trust carries only if the old folder was trusted.
  - A move keeps its agent; a missing or other-version CLI on the target refuses and sends Migrate (UP-1), which brings the older machine's CLI up, never down. The same holds for forks onto another machine and DK-16's machine picker.
  - Ask 4's "another channel" is CV-8, **Continue in another channel…**: a new top-level session set up as that channel's New session, whose agent resumes a copy of the conversation (from now or before turn n) under a new id, leaving the original untouched. Moving a whole channel session to another channel stays q11.
- **Review:** an Opus 5.5 xhigh judge reviewed these picks on 2026-11-16; its changes are applied above and in CV-1–CV-8 and q11. Still provisional.
- **Reverse:** a version allowlist entry per CLI; a byte-copy fallback could be added behind a setting.

## Frames for ROAM (to the owner through main; main's numbers 87–95 for the first nine)
87. Inbox card: agent proposes a repo for the channel (CH-26), with the "Also add it to <session>" tick.
88. Inbox card: move a Desk session into a channel (DK-16), with the "Also adds <repos> to the channel" line and the machine picker shown.
89. Move into a channel… dialog (user-initiated, editable branches).
90. Refused move: unknown conversation format, with Start fresh with a summary (CV-3).
91. Channel page Desk tab, with sessions and the empty state; frame 18's tab row gains Desk · n.
92. New desk session for a channel or session (CD-6), with the switches.
93. Channel Desk Info panel switches, one shown "Blocked by the channel rule: Never".
94. Channel Desk close confirmation listing Markdown with Copy to shared/ (CD-4).
95. Sidebar: a channel's Desk row (CD-2).
- 96: The target session's Info panel listing the Desk sessions that target it (CD-2).
- 97: The Continue in another channel… dialog (CV-8).
- (existing number, re-render) Re-recorded Agent access tab with the new IN-17 rows ("Stop or archive another session", "Delete another session").
- (existing number, re-render) Re-recorded Delete channel dialog (CH-24), listing the channel's Desk sessions.
Fork dialog turn lists for Codex/Copilot use FK-3's existing empty state, so no new frame. Move progress reuses MV-4.

## D308: A host resends a start the link dropped (STOP manager pick, 2026-11-16; D282 c addendum). Provisional
A box re-execs as it applies a stop, so a start the host sends can ride the link that is closing, or its answer can be lost. When a start ends Cancelled or Unavailable, the host waits for the box's link to change and sends it again, within D282 c's 20 s. A start is safe to repeat: a box already started answers FailedPrecondition ("not stopped"), which counts as done. Any other refusal is reported as is.
- **Reverse:** one send, and report the connection loss.

## D309: The one-off `humatad2 stop` sweeps as the daemon does and leaves its count for the next start (STOP manager pick, 2026-11-16; D269, D282 b addendum). Provisional
When the daemon doesn't answer, `humatad2 stop` runs the same sweep (D269) itself and writes what it ended to `<state>/stop/ended_early.json`. The next boot adds those holders to its own count in the start report, then deletes the file. An unreadable file counts as none, so a report may undercount but a start never fails on it.
- **Reverse:** the CLI ends holders without a record; the report then shows 0 for them.

## D310: A stream that ends by itself is not cut while the daemon leaves (STOP manager pick, 2026-11-16; D270 addendum). Provisional
A call whose stream ends by itself when the daemon leaves marks its response `EndsByItself`; today only the event feed does. A marked stream is left to end cleanly and is cut UNAVAILABLE only if still open 2 s after the daemon started leaving, inside the 3 s shutdown grace. Every other stream is still cut at once (D270). This replaces a fixed 250 ms wait, which loaded machines outran.
- **Reverse:** drop the marker, and the feed's subscribers may see UNAVAILABLE on a stop.

## D311: A repo with no default branch is refused in its own group, not the whole Diff panel (VIEWS manager pick, 2026-11-16; D293 addendum). Provisional
With per-repo bases, refusing all of GetDiff when one repo has no default branch hides the other repos' diffs and this repo's own base picker. The person then can't pick a branch or Uncommitted for just that repo. Instead, GetDiff should return a per-repo error for that repo. The panel shows the refusal (with the `git remote set-head origin --auto` fix) inside that repo's group and keeps its picker live. The other repos show as usual. This is a follow-up after VIEWS's UI lands. Until then frame 98 draws today's whole-panel refusal.
- **Reverse:** keep the whole-call refusal (D293 as written). The fix still works, but the other repos stay hidden until it is run.

## D312: A Terminal session's shell is the daemon's agent `zsh` (TERM manager pick, 2026-11-16; SPEC §1 agents table, flow 9 steps 3–4). Provisional
The app starts a Terminal session as the agent `zsh`, its command in SPEC §1, and the daemon knew only `claude`, `opencode` and what an agents file names, so every Terminal session and Open a shell was refused. The daemon now always has a kind `zsh`. It runs the shell that new shell tabs run (`--shell`, else `$SHELL`, else `/bin/sh`, with `--shell-arg`), whatever that shell is. It has no turns (status Idle while it runs) and no conversation, and it resumes as a fresh shell in its folder. An agents file that names `zsh` keeps its own. Open a shell still makes a Terminal session in a channel. A shell tied to no channel, in the machine's home folder, needs `OpenMachineShell` in the app and a place for such a tab in the model. That is still not built (HO-6's "Not wired yet" list).
- **Reverse:** map the app's Terminal to a name the daemon lists, and refuse Terminal when it lists none.

## D313: A session's folder follows the session on every move (owner decided, 2026-11-16; SPEC MV-16, DK-17, CD-1, MV-13, CV-4)
The owner wants agents, their context and their sessions to flow freely between machines and paths, and to resume trivially wherever they land. So where a session lives decides where its folder is:
- a Desk session's folder is in the Desk folder;
- a channel's Desk session's folder is in that channel's Desk folder;
- a channel session's folder is under the channel.

Every move goes through the same steps. It relocates the folder and its contents. It rewrites Humata's recorded paths and the conversation's paths. It resumes the agent at the new path. The old copy goes to the trash. This applies to Desk to channel, a channel rename or restore, machine to machine, and conversation roam. A move that leaves the folder or any path at the old place is a bug.
- **Reverse:** none; the owner's rule.
- **Addendum (owner, same day; SPEC MV-17):** Desk sessions and channel Desk sessions can be created on any linked machine, and any session can be moved to any machine at any time. Moves are cheap. Content-addressed chunks travel only when the target lacks them. Clones fetch on the target, never copied as `.git`. Progress is shown per item. The session is never live on two machines at once, and a move that is cut off finishes or undoes from its manifest.

## D314: A channel rename moves its folder (ROAM manager pick, 2026-11-16; MV-16 against FC-1). Provisional, asked in the wave
FC-1 kept the channel folder's name on rename. MV-16 says a session's folder follows where it lives, and the owner wants paths to follow names. So a rename now renames `<root>/<channel>/` once, then re-places every session of the channel through MV-16's steps: the recorded paths, the agent launch and the conversation. Every session stops before the rename and resumes after it, as a move does. A rename that would cut off a running turn asks first.
- **Reverse:** keep FC-1. The folder keeps its old name, and only the shown name changes.

## D315: Moving a Desk session with a picked folder moves that folder too, after the user agrees (ROAM manager pick, 2026-11-16; MV-16 against DK-2/DK-3). Provisional, asked in the wave
DK-2 and DK-3 never moved a picked folder. Under MV-16, a Desk session moved into a channel lands under the channel. When the folder was picked rather than made by Humata, the move dialog says the folder will be moved from `<picked path>`, and the user must tick that before Move is enabled. Nothing is left behind except the trash item.
A move to another machine copies or fetches the picked folder to the same logical place there; the source stays where it was until the move has been checked, then goes to the trash. Main is asking the owner about this pick.
- **Reverse:** a picked folder stays put. The session moves into the channel but keeps working at the picked path, which breaks MV-16 for that case.

## D316: A channel's Desk session can live on and move to any machine (ROAM manager pick, 2026-11-16; MV-17 against CD-1). Provisional
CD-1 said a channel's Desk session never changes machine. MV-17, which is the owner's words, replaces that. It is created on a picked machine and moved like any session, and it lands at `<root>/<channel>/desk/<n>` on the target. CD-1's text is amended to point to MV-17.
- **Reverse:** none planned; this is the owner's rule.

## D317: A peer link whose machine daemon is gone ends, so the host dials a new one (STOP manager pick, 2026-11-16; amends D213 and AWAY T3). Provisional
D213 kept a machine's link up across its daemon's restarts, so the host's Uplink outlived the daemon. But nothing carries over that link once the daemon behind it is gone. The host showed the machine online, yet every relayed call failed and no fresh report came until Reconnect. That left a stopped-then-started box at MachineStopped. Now the link ends whenever the daemon behind it ends, whether from a stop, a start, an upgrade or a crash, and the host redials after its 5 s backoff and gets a fresh Hello and report. A restarting box shows offline for about 5 s.
- **Reverse:** keep the link and have the host re-handshake inside it. That needs a protocol change, and a link that can't relay still looks healthy until then.

## D318: A channel's Desk rows stay in the main store and are rebased when the channel is restored (ROAM manager pick, 2026-11-16; MV-16, D314). Provisional
Restoring a channel under another folder must bring its channel-Desk sessions to the new place. Their rows stay in the main store, not the channel's side file, because a Desk item's "channel is gone" check finds the channel through them. `import_channel` rebases them, with their agent launches and earlier conversations, in the same transaction. What the user sees is the same as if the rows were snapshotted with the channel. A rename or restore while any session of the channel is Working, ProbablyIdle or NeedsInput asks first (`confirm_running`). An Inbox approval of an agent's rename counts as that confirmation.
- **Reverse:** snapshot the rows into the channel's side file, and find the channel for `before_restore` some other way.

## D319: A comment cut off mid-send ends Queued with its delivery unconfirmed, and the user is offered a resend (main's pick, 2026-11-16; SM-10, Gate 12 power cuts). Provisional
A power cut on the host after a comment's send was saved but before the terminal confirmed it leaves Humata unable to tell whether the text reached the agent. The comment comes back Queued with `delivery_unconfirmed` set. The user can send it again or take it back. Humata never types it again on its own, because typing it twice would reach the agent twice. The power-cut suites accept this outcome as "finished" for CommentSend: the comment isn't lost, and the user is told what's unknown.
- **Reverse:** confirm the delivery by reading the terminal's transcript after the restart and settle the comment as Sent or Draft. That needs a reliable marker in what the agent saw.

## D336: A moved session takes its own Inbox requests and summary with it; host-wide and PR-scoped rows stay (ROAM manager pick, 2026-11-16; MV-16, MV-17, D313). Provisional
When a session moves to another folder or machine, these rows move with it in the same import:
- its open Inbox requests (`inbox_requests`), so an ask from the agent can still be answered once it resumes;
- its AW-9 summary.
The request-hash dedupe table stays put, because dedupe is the host's job across every session. Review sessions and their PRs (`review_sessions`, `review_session_prs`) belong to the PR, not to one agent session, so they stay; any reference they hold to the moved session's id or path is rewritten. Nothing about the session is left at the old place except the trash item (D313).
- **Reverse:** leave the Inbox requests at the old place and close them as "moved", which asks the user again after the move.

## D337: A keyed Type is typed at most once, across restarts; one a crash cut short says its text may have been typed (TERM manager pick, approved by main, 2026-11-16; GRPC.md 5.2, coach bug 17). Provisional
Type notes its idempotency key in a new store table, `typed_keys` (schema step 28), before any text goes to the shell: pending first, typed once all of it is there. A retry the daemon's memory can't answer, as one after a restart, reads the row. Typed answers `typed` and types nothing. Pending means an earlier call started and never finished, as when the daemon stopped part way, so the retry fails saying some or all of the text may have been typed, and nothing is typed again; the person looks at the terminal and types what's missing with a new key. A call that typed nothing (the tab is missing, its shell exited, the holder refused or couldn't be reached) forgets its key, so a retry types. A timeout or a link lost part way leaves the key pending. Keys are kept 30 days, as every key is, and older rows are dropped when the daemon starts. A second call with a key whose first call is still typing gets the same may-have-been-typed answer. The rows carry no session, so they don't move with a channel; they belong to the call, not to a channel.
- **Reverse:** drop step 28's table in a later step and the claim in `type_in`; a retry after a restart types again, as before.

## D338: A shell tab is named by its program's title, and a new shell starts in the folder the tab is in (TERM manager pick, 2026-11-16; D73 panes, the TERM known limit on titles). Provisional
Not built. `PaneEvent::TitleChanged` (OSC 0/2) renames a shell tab the user has not renamed. The title loses its control characters and is cut to 80 characters. A tab the user renamed keeps its name, and so does the agent's terminal, which shows the title only as its tooltip. `PaneEvent::PwdChanged` (OSC 7) records the tab's folder, but only when the URL's host is empty or is the session's machine; any other host is ignored. ⌘T and **+** › zsh start the new shell there, and fall back to the session's folder when the tab has none or the folder is gone. Neither the title nor the folder is saved: after a restart the tab shows its last saved name until the program sets the title again.
- **Reverse:** drop the two events again (`ui/src/terminals.rs`); tabs keep the names they open with, and every shell starts in the session's folder.

## D339: On a Mac, ⌘⌫, ⌘← and ⌘→ edit the command line (TERM manager pick, 2026-11-16; D73, README gap 1, micro-02). Provisional
Not built. In `terminal/view/src/keys.rs`, ⌘⌫ sends Ctrl+U (delete to the start of the line), ⌘← sends Ctrl+A (start of the line) and ⌘→ sends Ctrl+E (end of the line), as Mac terminals do. These are the bytes shells and line editors already bind. A full-screen program receives the same bytes, and most read them the same way. ⌘C, ⌘V and the ⌘ scroll keys keep their meaning, and other ⌘ keys still go to the app. Off a Mac nothing changes: Home, End and Ctrl+U already reach the program.
- **Reverse:** remove the three arms in `keys::intent`; ⌘⌫, ⌘← and ⌘→ are ignored again.

## D345: The machine counts the lines it replays on a resume, and the host's reconnected line carries that count (TERM manager pick, approved by main, 2026-11-16; SE-16, TR-11, coach bug 16). Provisional
SE-16's "reconnected; caught up N lines of output" always said 0. The host relays chunks without decoding them (TR-11), and it can't tell where the replay ends and live output begins. The machine can: an attach from an offset the holder still keeps replays up to the end the holder had when it opened. It counts the newlines in those bytes and then sends a new `KIND_CAUGHT_UP` notice with `caught_up_lines` and `at_offset` set to that end. The count is sent even when it is 0. The host relay holds its `RECONNECTED` line until that notice arrives, sends the count on in it, and never forwards `CAUGHT_UP` itself. When the machine answers with a fresh screen instead (the offset was trimmed, or the resume was refused), the host sends `RECONNECTED` with no count just before the snapshot, and the app's line says the output is shown rather than claiming 0. Old machines are not supported: under force-upgrade both ends update together, so there is no fallback for a machine that never says `CAUGHT_UP`. The proto change is one enum value, so it is additive.
- **Reverse:** count in the app instead, from the output that follows the reconnect notice's `at_offset`. The app can't see where the catch-up ends either, so the count would include live output.
- **Open:** SE-16 also wants a fresh screen's line to say how many lines were not kept. The machine knows only the trimmed byte count, not the lines, so this waits for the holder to keep a line count with its trim point.

## D346: Moves skip whole files the target already has; chunk-level reuse comes next behind a proto change (ROAM manager pick, 2026-11-16; MV-17). Provisional
A move or Desk carry sends no file of 1 MiB or more that the target already holds by content. The target keeps a sha256 marker for each such file under its `.carry/have/` folder. `.git` is never copied: clones are rebuilt on the target from its main clone or the remote, plus a bundle of unpushed commits. Reuse is per whole file for now, so a large conversation that grew since the last move is sent again in full. The next step is a chunk index the target answers (a new transfer RPC, which needs a proto change), so that only the changed chunks travel. MV-17's "chunk dedupe" is met only once that lands.
- **Reverse:** drop the markers and always send every file; correct, but slow on slow links.

## D347: A conversation carries whole: Codex moves in place and updates its thread row; Claude takes its config-dir files; another conversation stays (ROAM implementer pick, 2026-11-16; MV-16, CV-4, D336). Provisional
- **Codex, one machine.** A move within one Codex dir rewrites the rollout where it is (no `Exists` refusal) and updates that thread's row in `state_5.sqlite` (`cwd`, `rollout_path`, and with a new clone's head `git_branch` and `git_sha`), no other row. CV-4's "never opens a SQLite file" is amended to this one write; a missing index is left missing. `thread_history_1.sqlite` is not touched, so its byte offsets may be stale after a rewrite that changes a line's length; unproven (X2). Across machines nothing changes: the CLI's index is never written.
- **Claude.** `locate` lists only the conversation's own files: the transcript, `<id>/`, the folder's `memory/`, and in the config dir (`CLAUDE_CONFIG_DIR` when set) `file-history/<id>/**` (byte for byte), `todos/<id>-agent-*.json` and `plans/<slug>.md` (the slug from the transcript). Another conversation of the folder is never listed or carried. The folder is looked up as given, then canonical (`/var` is `/private/var`), and placed under the canonical form of a folder that exists. The slug and todo names are inferred, not proven by a capture.
- **Carry rewrite.** The folder (and its canonical form), then the old config dir and home, each to the new one.
- **Store, host take.** A host that already holds the session (`Take::AgentOnly`) takes only the agent's own rows: launch, interruptions, previous conversations, fresh seed. The Desk rows, fork origin, archive, comments, Inbox requests and AW-9 summary are the host's and stay. Inbox rows are carried whole, answered or not: the store can't tell open from answered, which is in the daemon-encoded entry.
- **Proven by X2 (2026-11-16).** Claude Code 2.1.326 names `projects/<folder>` by making each UTF-16 unit that isn't an ASCII letter or digit a `-` (two for a character outside the BMP); a name over 200 units is cut to 200, then `-` and the base-36 magnitude of the path's 32-bit `String.hashCode`. The adapter and `{folder_dashed}` share one function for it. Codex 0.169.0 never repairs the byte offsets `thread_history_1.sqlite` keeps into a rollout rewritten where it is, though resume still works; with the thread's rows gone it rebuilds them from the rollout. So a move within one Codex dir now first deletes that thread's `thread_items`, `thread_turns` and `thread_history_projection_state` rows (the CLI's trigger drops its realtime items), which replaces "not touched" above.
- **Reverse:** carry only the transcript and rewrite nothing outside the per-folder record, at the price of a resumed agent not finding its backups, todos and plan.

## D348: A channel rename reaches each linked machine at once or when it next links; a busy machine asks in the Inbox (ROAM manager pick, 2026-11-16; MV-16, D314). Provisional
Renaming a channel renames its folder on the host first, then on every linked machine the host can reach, by relaying RenameChannel; each machine derives its own folder from the new name. The host doesn't record machines that are offline. When one links, the host compares that machine's channel name and folder with its own and relays the rename; this survives a host restart. A machine whose agents are busy doesn't rename on its own. The host puts one Inbox request, "Rename <channel> on <machine>", with the sessions that would be stopped; approving it counts as `confirm_running`. Until then the host's records of that machine keep its old path, so nothing is shown that isn't true there. Closing, cleaning up or sweeping a remote Desk session needs its machine and is refused with `machine_offline` while it's away, like stop and resume.
- **Folder clash and memory.** A machine that already has a folder with the new name asks the same way: it picks no other name and leaves the folder alone, and approving retries with `confirm_running`. These requests live in the host's memory, not the store; after a host restart, the link-time catch-up raises them again, and one is withdrawn when the machine catches up another way.
- **Reverse:** queue every remote action as replayed work and show it pending, instead of refusing while the machine is away.

## D349: A Claude conversation is written for the target's folder before it is chunked (ROAM implementer pick, 2026-11-16; MV-16, MV-17, D346). Provisional
Unpacking used to rewrite the old folder's path in the conversation, so the chunks the old machine listed (its paths) never matched the ones the target held (its paths), and a moved conversation was sent whole every time. Now the host asks the target for the session's folder (`SessionCarryService.Target`), `Pack` carries it, and the old machine has the adapter place the staged Claude files for it into a scratch home, then names the old home and config dir back, so unpacking (which knows the new home) still rewrites those. The manifest records the folder as `placed_for` (an old manifest has none and is placed from the old folder as before). The old machine stores the chunks of the staged conversation in its own form before rewriting it, and the new machine those of the transcript as placed, so a session that goes away and comes back finds its chunks. Only Claude is placed ahead; Codex and Copilot carry as before, so their moved conversations keep sending whole files whose lines name the folder.
- **Reverse:** drop `target_folder` and unpack from the old folder; correct, but a moved conversation is sent whole.

## D320: Off a Mac, every ⌘ shortcut is typed with Ctrl+Shift (BUGS manager pick, 2026-11-16; flows/cases bug 18, NV-6, KP-2). Provisional
On Linux the app's ⌘ chords were bound to Super, which the desktop keeps, so no shortcut worked. Off a Mac each ⌘ chord becomes Ctrl+Shift; one that already holds Shift or ⌥ becomes Ctrl+⌥+Shift, so ⌘N and ⇧⌘N stay apart. Plain Ctrl stays the shell's: bindings are matched before a terminal sees the key, and Ctrl+K, Ctrl+L, Ctrl+N and Ctrl+[ mean something there. Labels in the palette and the Shortcuts sheet still read ⌘ (follow-up), and the desktop tests that press `secondary-k` need `ctrl-shift-k` on Linux.
- **Reverse:** bind the ⌘ chords to plain Ctrl in `ui/src/keys.rs` `off_mac`, and accept that they shadow the shell.

## D321: ⌘0 and ⌘L dock a folded sidebar when the window has room, and show it over the page only when it has not (BUGS manager pick, 2026-11-16; bug 20, NV-6). Provisional
`Layout::reveal_sidebar` says how a hidden sidebar comes back: Dock when only the user's fold hides it, Overlay when the window is too narrow. ⌘L shows the sidebar the same way before focusing it, so focus never lands on rows that are not drawn. ⌥⌘M with no session open shows a toast that says to open a session first, instead of doing nothing (bug 24).
- **Reverse:** make ⌘0 always open the overlay while the sidebar is hidden, as before.

## D322: The side panel's fold is kept per session, for this run (BUGS manager pick, 2026-11-16; bug 25, NV-37, SE-13). Provisional
Folding the side panel in one session leaves the others as they were; the fold is not saved across restarts.
- **Reverse:** keep one app-wide `layout.panel.collapsed` again.

## D323: The archive toast's Undo lasts the daemon's 10 s undo window (BUGS manager pick, 2026-11-16; bug 28, MV-15). Provisional
The archive toast used the 4 s toast life, so Undo went away while the daemon still allowed it. Undo toasts now last 10 s (`UNDO_TOAST`).
- **Reverse:** use the 4 s life for every toast.

## D324: `session_type` types raw keystrokes (BUGS manager pick, 2026-11-16; bug 51, SE-23). Provisional
`Type` and the MCP `session_type` type exactly what they are given; the caller adds Enter, and wraps the text in bracketed paste itself if it wants one paste. The docs said otherwise and now match the code and its tests.
- **Reverse:** wrap in bracketed paste in the daemon `type_in` and change the tests that type Enter.

## D325: A queued multi-line message is sent as one paste; a one-line message is typed raw (BUGS manager pick, 2026-11-16; bug 62, SE-23). Provisional
A multi-line queued message is wrapped in bracketed paste so its newlines don't each send a turn; a paste-end marker inside the text is removed. One-line messages are typed and Entered as before.
- **Reverse:** always wrap in `clients/live/src/message.rs` `typed`.

## D326: Needs you is not a turn end; Probably idle is not sure (BUGS manager pick, 2026-11-16; bugs 59-61). Provisional
Live statuses map Working and Needs you to a running turn, Probably idle to an uncertain one that keeps queued messages waiting, and only a reported idle ends the turn. OpenCode's hook-backed turn end now delivers queued messages.
- **Reverse:** map Needs you and Probably idle to Turn::Ended in `clients/live/src/map.rs`.

## D327: Declining a CLI "Continue?" exits 13 (`declined`) (BUGS manager pick, 2026-11-16; bug 53, api/CLI.md). Provisional
A no is a deliberate answer, not an interrupt, so it no longer shares 130 with Ctrl-C.
- **Reverse:** map `NotConfirmed` back to the interrupted exit and drop row 13.

## D328: The approval passphrase is typed only at `humata login` (BUGS manager pick, 2026-11-16; bug 54, SEC-13). Provisional
The daemon never read a passphrase sent with an Inbox answer. The CLI prompts for it only when logging in; any other call refused for presence says to run `humata login`. A wrong passphrase reports the tries left (bug 56).
- **Reverse:** check `presence.approval_passphrase` in `answer_request` and let `Ctx::proof` prompt for any call.

## D329: A CLI wait that ends with no answer is `pending` (exit 12) (BUGS manager pick, 2026-11-16; bug 55). Provisional
The request is still waiting, so the CLI says how to wait again instead of blaming a version mismatch.
- **Reverse:** return `DaemonError::Ended` at the two sites in `cli/src/call.rs`.

## D330: The CLI shows a machine copy's age (BUGS manager pick, 2026-11-16; bug 67). Provisional
A read answered from a machine's copy carries `stale_as_of` in the `--json` envelope and a stderr line otherwise, as MCP does.
- **Reverse:** drop `stale_as_of` from the CLI `Daemon` and `output::finish`.

## D331: MCP lists only the tools the daemon runs, and its refusals say what to do (BUGS manager pick, 2026-11-16; bugs 48-50, 52, 64). Provisional
`tools/list` is what the daemon dispatches. A fork's reason travels in `ForkSessionRequest.reason` onto the card. A stopped machine's refusal says to ask the user to start it. A turn-end `session_type` and a fork to a machine this host doesn't know are `invalid_field`, not "not built".
- **Reverse:** let `Daemon::serves` default to true with no filter, map the refusals back to "not built", and drop `reason = 10`.
- **Note (daemon triage, 2026-11-16):** a Desk start on a machine this host doesn't know is likewise `invalid_field` on `machine` with problem `UnknownValue`, as a fork's is, not `not_found`.

## D332: The after-restart note makes no claim about a turn in progress (BUGS manager pick, 2026-11-16; bug 29). Provisional
The daemon has no per-session mid-turn fact, so the note says the session came back idle and to tell it to continue if it was working.
- **Reverse:** add a mid-turn flag to `LastStart` and restore the old line where it is set.

## D333: A changed host key is cleared by removing and re-adding the machine (BUGS manager pick, 2026-11-16; bug 37). Provisional
Until TrustNewHostKey is built, a key that differs from the pinned or confirmed one never gets a Confirm card; the message says to remove the machine and add it again, checking the new fingerprint.
- **Reverse:** build TrustNewHostKey and show the pinned key beside the new one on the card.

## D334: The board's Status menu offers the five columns; pins of deleted sessions go when the list is read (BUGS manager pick, 2026-11-16; bugs 45, 47, BD-1). Provisional
Blocked and Won't do have no board column, so the menu leaves them out; an item that already has one still shows it. A pinned session the store no longer has is dropped when the Pinned list is read.
- **Reverse:** add the two columns to `BoardColumn` and remove the filter; prune pins from the real delete paths instead of at list time.

## D335: A terminal opens only web and mail links; pasted images and files reach the session's machine (TERM manager pick, approved by main, 2026-11-16; SE-16, FM-4, FM-10, coach bugs 4 and 5). Provisional
A link clicked in a terminal opens only when it is `http`, `https` or `mailto`; any other scheme stays closed and a toast says to copy it and open it by hand if it is trusted. A pasted image goes through `PasteImage`: the daemon saves it in the session's paste folder and its path is typed. Files pasted from Finder into a session on this Mac type their own paths, each as one shell word. Files pasted into a session on another machine are uploaded through the existing file transfer into the session's pastes there, a new `paste-<id>` folder for each paste so names never clash, and the paths on that machine are typed. The pastes are a new file root, `SessionPastes` (files.proto, the user only like home: no agent reads or writes it and no grant covers it), under the store's `files/<session>/paste/`, so they go when the session does. The upload's answer carries the on-disk path in a new `Transfer.disk_path`, set only for a done upload into the pastes; for another machine the host asks it with the new `TransferEndService.Locate`, since the host knows only the engine's virtual path. That keeps one call per file, and `FileEntry` is unchanged. Anything that isn't a file is refused before anything is sent.
- **Reverse:** drop the `SessionPastes` root, `disk_path` and `Locate`, and go back to the toast that says to copy the files to the machine first.

## D340: A queued message is keyed by the app clock, a run serial and its text (BUGS manager pick, 2026-11-16; bug 63, follow-up). Provisional
The key that keeps a re-sent queued message from being typed twice is `queued-<session>-<clock nanos>-<serial>-<position>-<text hash>`, made from the clock client, not `SystemTime`, so the reducer stays pure and its tests use `TestClock`. The app clock starts over each run, so the serial keeps same-instant messages apart within a run and the text hash keeps a new message after a restart from reusing an old key.
- **Reverse:** make the key from `SystemTime` again in `QueuedMessage::new`.

## D341: A restart cues the app with one Resync (BUGS manager pick, 2026-11-16; bug 32 leftover). Provisional
After the restart report is set, `after_restart` publishes one `Resync`, so a restart with nothing to resume still makes the app read the report. It costs one extra list reload per restart.
- **Reverse:** remove the publish in `session_service/watch.rs` `after_restart`.
- **Engineering fix (2026-11-17, no product change):** the Resync goes out through `Feed::resync_all`, unnumbered and not kept, as events.rs documents for every Resync. It had gone through `publish`, so it was numbered and kept, and a client resuming from a later point replayed a stale one. Subscribers connected at the restart still get it. Test: `humatad2_unit_test` `a_resuming_subscriber_does_not_replay_an_earlier_resync`.

## D342: The palette's Files load once per session, when the palette first opens (BUGS manager pick, 2026-11-16; bug 23, replaces nav_2). Provisional
Opening the palette runs one FindFiles (at most 500 entries) for each non-archived session not asked yet; workspace loads and Resyncs fetch nothing and keep lists already fetched. A failed fetch is retried at the next open. After that the daemon pushes `FileChanged` (events.proto 31): the files service tells each user write, rename and trash, and the pointer loop's clone watcher stats only the paths each debounced notify batch names and tells adds, removes and 1:1 renames (at most 500 per batch; content-only edits tell nothing). A clone is listed whole only at first sight and on a notify rescan; a clone over 50,000 files is left alone until a rescan. The app applies them only to sessions already fetched. An edit made outside Humata to a session with no live agent is not told.
- **Reverse:** fetch at workspace load (nav_2), or drop the `FileChanged` publish and re-fetch at each palette open.

## D343: The user's own edit through the files service does not address a Sent comment (BUGS manager pick, 2026-11-16; bug 42). Provisional
A user WriteFile into a session clone rebaselines Sent comments on that file: they stay Sent and their anchors follow the new text. Only changes that do not come through the files service (the agent or a terminal writing the disk) address a comment. Draft and Waiting comments still detach on any edit. A write forwarded to another machine is not hooked yet.
- **Reverse:** remove the `use_user_writes` line in `serve.rs`; every edit counts as the agent's again.

## D344: Off a Mac, shortcut labels read Ctrl+Shift (BUGS manager pick, 2026-11-16; bug 18 follow-up, completes D320). Provisional
The Shortcuts sheet and the palette hints show the chord a binding is typed with: `⌘K` reads `Ctrl+Shift+K` and `⇧⌘N` reads `Ctrl+Alt+Shift+N` off a Mac; labels without ⌘ and every label on a Mac are unchanged. The desktop tests press `cmd-` on macOS and `ctrl-shift-` elsewhere.
- **Reverse:** show the Mac glyphs on every platform (`keys::label` returns its input).

## D380: The app reads the Desk again after a Desk change and on opening the Desk page (WIRE-A manager pick, 2026-11-16). Provisional
The daemon sends no Desk events, so live the folder bar, Closed rows, Documents and Clean up stayed stale. The app now reloads the workspace after a successful Desk start, change (close, reopen, keep, repos, access), copy or clean up, and when the person navigates to the Desk page. It is not a poll: each reload follows something the person did. A document an agent writes while the Desk page stays open shows on the next visit, not at once.
- **Reverse:** add Desk change events to the daemon's event stream (a proto SEAM) and drop the reloads in `features/app/src/desk.rs` and `route.rs` `desk_fresh`.

## D381: Typing an ask into a session's queue is logged as Not built (WIRE-A manager pick, 2026-11-16). Provisional
Flow 05 step 6 has the person type two asks while the agent works and see them wait under Waiting to send. The queue itself works live (`desktop/tests/live_flows/queue.rs`), but no control in `ui/src` sends `SessionAction::Draft` or `Submit`: keys typed in a session go to its terminal. The queue is fed today only by app actions (tests) and by comments. WIRE-A tags the typing as Not built and leaves where an ask is typed (a box under the terminal, or a key that holds the next line) to whoever designs it; the flow keeps its words.
- **Reverse:** add the entry control, then retag flows/05 step 6 to its test.

## D382: A Desk document needs a file root for the session's own folder (WIRE-A manager pick, 2026-11-16). Provisional
A Desk session has no repo: its document sits in the session folder, outside every clone. `FileLocation` names only clones, the main clone, a channel's shared or own folder and home, so the live client sends a session clone with an empty repo (`clients/live/src/diff.rs:145`) and the daemon refuses it (`daemon/humatad/src/files/roots.rs:364`). Comments already anchor there with an empty repo (REVIEW t8). Pick, taken by main 2026-11-16: add a `SessionFolder { SessionId session = 1; }` root to `FileLocation` (field 9; TERM's j1 takes 8), served under the same policy checks as a clone, and have the live client use it when the repo is empty. It is a proto SEAM for main, so flows/04 steps 3 to 8 on the Desk are tagged Not built: WIRE until main takes it; the clicks are proven on a channel session meanwhile.
- **Reverse:** keep Desk documents unreadable live, or copy them into a clone first.
Built: the daemon places it as the session's own clone root, so access follows the session's grants exactly as its clones do (the owner's agent and its parent read and write; channel-mates need FilesReadOtherSession; a Desk agent against another session is refused). A grant on a clone path such as `acme/api/...` does not cover the same files through the folder root; a whole-session grant does. The MCP `files` tools and the CLI's locations don't name the root yet.

## D383: A drawer move carries the item's revision; a stale one is refused (WIRE-A manager pick, 2026-11-16; reversed the same day by main). Provisional
First logged as Not built: the client `Item` had no revision and `clients/live/src/boards.rs` sent `expected_revision: None`, so a drawer move made while an agent moved the same item won silently. Main ruled that a bug. Now `Item` carries `revision`, `Daemon::move_item` takes the revision the caller last read, and the live client sends it as `expected_revision`; the daemon's refusal reaches the app as `DaemonError::Conflict`, and the board shows why, reads the board again and keeps the drawer open. Adding or removing a dependency and setting an item's session still send no revision: each adds or drops one member and overwrites nothing. A title save stays Not built: the drawer has no title edit (a board-edit milestone).
- **Reverse:** send `expected_revision` on the dependency and session patches too (`patch_item` already takes `seen`).

## D384: Turning the wheel over the page in Side by side unticks Link scrolling (WIRE-A manager pick, 2026-11-16). Provisional
SPEC says the page follows the source while scrolling is linked, and flows/04 says scrolling one side by hand unticks it. Only the checkbox unticked it. The page is the side that follows, so a wheel over the page is the hand scroll and unticks; a wheel over the source scrolls both and stays linked.
- **Reverse:** untick on a wheel over either side, which leaves the link nothing to follow.

## D385: SessionFolder reaches agents, the CLI and the file manager; no visible Open folder control yet (WIRE-A manager pick, 2026-11-16). Provisional
D382's root is now `session_folder` in the MCP files tools (session defaults to the caller's own; access follows the session's grants), `<machine>:folder/<session>/<path>` in the CLI (a session named `folder` would read as this word, as `shared` and `main` already do), and `FileRoot::SessionFolder` in the file manager. `DeskAction::OpenFolder` opens a Desk session's folder in the Files page's active pane. No button calls it yet: an Open folder control on the Desk folder bar changes the native snapshots of frames 36 to 40, which must match the design goldens, so it waits for a golden pick.
- **Reverse:** add the control once a golden shows it, or reach the action from the palette.

## D386: The Desk form's preview counts a folder the daemon reports by its full path (WIRE-A manager pick, 2026-11-16). Provisional
The form previews `~/Humata/desk/<name>` and suffixes a taken name. The live daemon reports each Desk session's folder as an absolute path, so no record ever matched and the live preview never showed the suffix the daemon then gave. A recorded folder now counts as taken when it equals the preview or ends with the preview's path after `~/Humata/`.
- **Reverse:** have the daemon report the folder in the `~/` form the form uses, or have the form ask the daemon for the next free name.

## D387: A session carries its pin and VIP mark from the daemon (WIRE-A manager pick, 2026-11-16). Provisional
`session_pb` left `Session.pinned` and `Session.vip` unset, so only the marks client saw them. They are now kept per session in the daemon's process-wide `read_marks` store beside the unread set: loaded from `<state>/marks.db` at start, updated by MarkService.SetMarks. A real change goes out as a Session Updated event through the existing read-mark path. It covers this daemon's own sessions; a session placed on another machine shows the host's mark.
- **Why provisional:** `read_marks` is now a little wide for its name, and a remote session's marks aren't read from its machine.
- **Reverse:** move the pin and VIP sets into a module of their own with their own change channel; the tests go through MarkService, SessionService and the feed, so they stay.

## D388: Each repo group's Push & PR button is named for its repo (WIRE-A manager pick, 2026-11-16). Provisional
Every group drew a Push & PR button with the same name, so a click by name couldn't pick one. Its accessible name is now `Push & PR: <repo>` (for example `Push & PR: owner/ledger`); the drawn text stays, so the goldens don't change. A screen reader now says the repo too.
- **Reverse:** drop the accessible name and click the buttons by id (`changes-push-<repo>`), as the fake-client tests do.

## D389: The live rig waits on turn ends, not on the conversation file, where the daemon can tell it (WIRE-A manager pick, 2026-11-16). Provisional
A test that needs the agent to have taken a message runs the agent with turn hooks and waits on the daemon's Session events for a turn end, reading the conversation once per event. `running_session` still re-reads the conversation every 100 ms: the live event stream carries no terminal output, and its callers' agents report no turns.
- **Reverse:** add terminal output to the live event stream (events.proto, a SEAM) and wait on the agent's `got:` line everywhere.

## D390 (Provisional, WIRE-B): a restart already known at the first load is told once per app run

- **Context:** when humatad brings its sessions back before Humata opens (it starts at login), the app's first load already holds the restart, so no Restart event comes and no toast was shown (flow 7 step 3).
- **Pick:** the app toasts each restart a load brings that it didn't hold before the load (same machine and start time). A reload tells nothing again. Quitting and reopening the app tells the latest restart again, once.
- **Why provisional:** remembering across app runs which restarts were told (in the saved app state) would stop the repeat after a reopen; it's reversible and waits on the owner.

## D391 (Superseded by D392, WIRE-B): the live read mark sends nothing until the daemon keeps read marks

- **Context:** opening a session that is unread or needs recovery sends `mark_read` (features/session). The live client answered it "not in this release", so opening a session that couldn't resume after a reboot showed an error (flow 7 step 5).
- **Pick:** the live `mark_read` succeeds and sends nothing. The daemon sets no unread mark today (CH-17 bold-until-opened isn't built on the daemon; the proto's `unread` is never set), and a session that couldn't resume keeps its recovery until Try again or Start fresh, which is the card. The fakes still clear both.
- **Why provisional:** real read marks need a SessionService call to clear `unread` (a proto SEAM) and the daemon to set it on new output. Not built, no milestone; reversible.

## D392 (Provisional, WIRE-B): the daemon keeps read marks; a client holds the session it shows viewed

- **Context:** CH-16 shows a session with unread output bold; CH-17 clears it when the session is opened. The daemon never set `Session.unread`, and the live `mark_read` sent nothing (D391), so the fakes and the real daemon differed. The proto's ActivityService.MarkSeen was never built.
- **Pick:** the daemon marks a session unread when its agent prints while no client views it, and keeps the mark in `<state>/marks.db` (an `unread` table beside the pins), so a restart keeps it. A client views a session by holding MarkService.ViewSession open: opening it clears the mark (the first message says whether it had one), and output while any client holds one marks nothing. Each change is a Session event. The app tells the daemon which session it shows on every navigation (`Daemon::view`); `mark_read` views the session too. MarkSeen is dropped: a one-shot clear would let the open session turn bold under the user's eyes. The fakes do the same, proven by the parity cases; a session that couldn't come back keeps its card when opened on both.
- **Limits:** only output the daemon follows marks a session, so a session running on another machine isn't marked on the host yet. A terminal attach is not a view, because the app keeps links up for hidden sessions. The app's window being in the background still counts as viewing.
- **Why provisional:** the owner may want per-client or per-user marks, or marks for shell tabs too; all are reversible behind the same call.

## D393 (Provisional, WIRE-B): a resize or attach redraw never marks a session unread

- **Context:** the app keeps links up for hidden sessions and resizes them with the window; the program redraws on SIGWINCH, and under D392 that redraw marked the session unread.
- **Pick:** after the daemon resizes a session's agent tab (an attach's first size or a later resize), the session is redrawing until something is typed to that tab or the agent prints a word it hadn't printed in its recent output (the last 64 KiB of text, escape codes stripped, split into words, words with no letter or digit ignored). While redrawing, output with no new word marks nothing; new output still marks. The window lives in the daemon's follow loop (`read_marks/printed.rs`).
- **Limits:** a repaint with changed words (a clock, a timer, a percentage) still marks. Genuine output made only of words already in the window doesn't mark while redrawing. The window starts empty when the follow starts (after a daemon restart), so the first redraw after it can still mark. Terminal-query answers don't count as typing.
- **Why provisional:** a screen-level comparison on the tab's authoritative screen (D271) would be exact but costs a text read of the screen per chunk; reversible behind `read_marks`.

## D394 (Provisional, WIRE-B): a remote session's read mark is its machine's, shown on the host

- **Context:** D392 marked only sessions whose output the daemon follows, so a session on another machine was never unread on the host.
- **Pick:** the machine marks its own sessions as D392 says and reports the mark on the Uplink (`SessionStatusReport.unread`, SEAM); the host's merged view shows it. ViewSession on the host for a session placed elsewhere answers at once from the host's view and holds MarkService.ViewSession open on the machine through the relay (added to the relay allowlist), reopening it whenever the machine's link comes back, pushed, while the client's view is held.
- **Limits:** the host doesn't save the mark (a new column in `remote_session_reports` is a store SEAM); after a host restart it shows again from the machine's first report. While the link is down, a viewed remote session stays unread until the link returns.
- **Why provisional:** the owner may want the host to keep the mark itself; reversible.

## D395 (Provisional, WIRE-B): lost read-mark changes make every client reload

- **Context:** the daemon's read-mark changes reach the feed through a bounded broadcast; a lag dropped them, so clients showed stale marks until their next load.
- **Pick:** on a lag the daemon sends a Resync to every feed subscriber (`Feed::resync_all`, unnumbered and not kept for replay), and clients reload at once, as they already do on a Resync.
- **Why provisional:** re-telling only the sessions whose marks differ would be lighter; reversible.

## D396 (Provisional, WIRE-B): the host keeps a remote session's read mark with its last report

- **Context:** under D394 the host showed a machine's mark from its reports but didn't save it, so after a host restart a remote session showed read until the machine reported again, and stayed read while the machine was down.
- **Pick:** `remote_session_reports` gets `unread INTEGER NOT NULL DEFAULT 0` (store step 30, after TERM's step 29; SEAM); the host saves the mark with each report and rebuilds its views from it at start, so a restarted host shows the mark at once. The machine's next report still overrides it. Reports kept before the step read as not unread.
- **Limits:** the kept mark is as of the last report; if the session was viewed on the machine while the link was down, it corrects at the next report.
- **Why provisional:** the owner may want the host to hold read state of its own; the step number may need moving if another lane adds a store step first.

## D397 (Provisional, WIRE-B): the desktop live rig can link a second machine

- **Context:** the desktop live rig used `FakeMachines` with no linked second machine, so the app side of flow 08 step 11 couldn't run live.
- **Pick:** `World::start_with_machine()` starts devbox-1, a fake machine behind a fake ssh with its own machine-role `humatad2` and agent, linked to the host, with the app on the real `LiveMachines`; `Devbox::{cut_link, restore_link}` take the link down and back. `live_flows/away_box.rs` drives a request answered on devbox-1 while the link was down. `start()` is unchanged.
- **Limits:** the app lists waiting requests only and its `Request` has no field for where it was answered, so the test proves the link, the host Inbox and that the app never holds the request as waiting, not an "answered on devbox-1" row. Its waits are pushed (devbox-1's Inbox and Machines feeds, a blocking fifo, the CLI's exit) except two, listed as push gaps in WIRE.md: the wait for devbox-1's socket to listen, and the app-state wait that settles the headless app. devbox-1 runs the admin build (`--admin-pid`, as the daemon tests do) so the test can read its user-only feeds.
- **Why provisional:** showing answered requests and where they were answered is a model, list and Inbox UI choice for the owner (no milestone yet).

## D398 (Provisional, WIRE-B): humatad2 says it listens: READY=1 on its ready pipe and to NOTIFY_SOCKET

- **Context:** whoever started `humatad2 serve` learned it listened by retrying a connect (the live rig, each 20 ms) or by a bare byte on `--ready-fd` (the bridge), and a systemd unit had no way to wait at all.
- **Pick:** once the socket is bound, and before the first call is answered, `serve` writes `READY=1` and a newline to the `--ready-fd` pipe and closes it, and sends the datagram `READY=1` to `NOTIFY_SOCKET` when that is set (a leading `@` is an abstract name on Linux), so LINUX's user unit can be `Type=notify`. The pipe is made close-on-exec as `serve` takes it, so holders and agents started before ready can't keep a starter waiting; `NOTIFY_SOCKET` stays in the environment, because systemd takes notices from the main pid only and an exec relaunch keeps that pid. A daemon that won't listen closes the pipe without a word. A notice the service manager refuses is logged as a warning, not fatal. The bridge, which reads one byte, is unchanged. The live rig's `World::spawn_daemon` and devbox-1 start on the pipe (`spawn_ready`), and `reboot` waits for each killed process's exit through kqueue, not a `pgrep` loop.
- **Why provisional:** a relaunch by exec before ready (an upgrade rollback at boot) drops `--ready-fd`, so its starter now hears the pipe close and counts the start as failed though the relaunched daemon comes up; carrying the pipe across that exec is a small follow-up if it matters.

## D399 (Provisional, WIRE-B): headless waits park until something wakes them

- **Context:** the live flows' `wait_until` settled the headless app, looked, and slept 100 ms. A watch on the store's state version can't serve as the signal: the store's state moves only inside `settle`'s `run_until_parked`, and terminal output reaches the views through gpui tasks, not the store.
- **Pick:** between looks the test thread parks (`std::thread::park_timeout` to the deadline). It is woken by gpui itself, whose test scheduler unparks the thread that made the app whenever another thread hands the app work, by every event from the host's feed, and by the agents: each writes a byte to the World's `wrote` fifo after adding to its conversation, and a thread unparks the test and fires `World::wrote()`. Waits run on the thread that opened the app, which is asserted. Two waits look at what nothing pushes and keep a 100 ms look (`App::wait_polling`, marked GAP): the host's history index reading a conversation, and a scratch folder going from disk.
- **Amended (live flows timeout fix):** the park is also capped at 16 ms, a display's frame tick. A terminal's pump that applied a full batch of a flood waits for the window's next frame, so frames that queue behind it wake no gpui task. A park to the deadline then drew no frame, and the pump, and the shell open waiting behind it, stalled until the wait timed out. The cap draws the frame a display would.
- **Why provisional:** a wait whose predicate reads something that none of the three wakes now times out at its deadline instead of passing late, which names the wait; the fix is a push for it or `wait_polling`.

## D438 (Provisional, WIRE-B): the history index says when it caught a session up

- **Context:** D399 left one live wait on a 100 ms look: whether the host's history index had read a conversation, which nothing pushed.
- **Pick:** each catch-up that adds lines (local or from a remote) publishes `HistoryIndexed { session, added }` on TOPIC_SESSIONS, scoped to the session's channel and the session; a catch-up that adds nothing publishes nothing. The client turns it into `Event::HistoryIndexed`, and a waiter searches again on it. The app ignores it for now (GAP): an open search is not rerun. Proven by `daemon/humatad/tests/history.rs::catching_a_conversation_up_is_told_on_its_sessions_feed`; `live_flows/find.rs` waits on it.
- **Why provisional:** the event carries a count, not the lines, so a client that wants results still searches; rerunning an open search on it is a view pick for later.

## D439 (Provisional, WIRE-B): an upgrade rollback that relaunches by exec keeps the ready pipe

- **Context:** with D398 the starter waits on `--ready-fd`; a rollback that relaunched by exec dropped the flag, so its starter waited out its deadline though the rolled-back daemon was up.
- **Pick:** the rollback clears close-on-exec on the pipe and passes `--ready-fd` through unchanged when it has not said READY yet, so the relaunched daemon says READY=1 on the starter's pipe; the upgrade's own exec and the relaunch after a stop have already said it and drop the flag. If the pipe can't be handed on, the daemon logs it and relaunches anyway. Proven by `tests/upgrade_cut.rs::a_rollback_relaunch_says_ready_on_the_starters_pipe` and unit tests in `ready.rs` and `stop/exec.rs`.
- **Why provisional:** a rollback to a release older than D398 would refuse the flag; today every rollback target is the same build line, and the UPGRADE milestone owns mixed versions.

## D440 (Provisional, WIRE-B): a Desk scratch folder is in the Bin before its row is marked deleted

- **Context:** D399 left the second 100 ms look: whether Clean up had taken a scratch folder off disk.
- **Pick:** keep the order clean-up already has and make it a rule: the folder moves to the Bin, then the row is marked deleted, then the event goes out after the commit. So once a client hears the session leave, its folder is gone, and the live wait is a single look. Proven by `daemon/humatad/tests/desk.rs::a_scratch_folder_is_in_the_bin_before_its_row_is_marked_deleted`, which stops the daemon at the row's commit and finds the folder gone.
- **Why provisional:** a crash between the move and the mark leaves a row for a folder in the Bin; the next clean-up marks it, which is today's recovery.

## D441 (Provisional, WIRE-B): live clone waits rely on the clone happening before launch

- **Context:** `files.rs`, `document.rs` and `review.rs` looked for a session's clone every 50 ms.
- **Pick:** the daemon records and clones before it launches the agent, so once the agent has the first message (pushed by the `wrote` fifo, D399), the clone is there and a single look finds it. `document.rs` uses `review.rs`'s `clone_of`.
- **Why provisional:** if starts ever launch before cloning, these looks fail at once, which names the change.

## D442 (Provisional, WIRE-B): the inbox, machines and boards live waits park

- **Context:** `live_inbox.rs`, `live_machines.rs` and `live_boards.rs` kept D399's old 100 ms `wait_until`, and the inbox and boards rigs a 100 ms loop for the agent's first message.
- **Pick:** the same as D399, copied into each (each test is its own crate): the waits park and are woken by gpui, by every host event, and (inbox, boards) by the agent's writes to a `wrote` fifo after each conversation line and each file it leaves in `out`. The first-message loops await `World::wrote()`.
- **Why provisional:** a copy per rig; a shared test module needs a BUILD change and can come with the next rig.

## D410: The stack CLI's user-facing picks (STACK manager, Opus advisor, 2026-11-16). Provisional
`bazel run //tools/stack -- <command>` makes each `Topic:` trailer group on a branch one PR, based on its `Relative:`
topic's PR (or the trunk). `stack commands` lists every command (`--json` for agents); `stack guide` is the manual.
Owner-visible picks:
- **Branches.** Head branches are `<prefix>/stack/<topic>`, flat (no nesting by Relative), and stay the same across
  amends; renaming a Topic opens a new PR. The prefix is `--prefix`, else `prefix` in stack.toml, else the gh login;
  it must pass `git check-ref-format`. An open PR keeps its head branch when the prefix changes. Remote branches are
  never deleted.
- **Config.** Settings are compiled-in defaults, overridden by `$HUMATA2_HOME/config/stack.toml` (else
  `~/.humata2/config/stack.toml`); nothing is read from git config, env vars (other than the home) or the repo.
  Unknown keys are refused; secret-looking keys (token, secret, password, credential, auth, api_key, ...) and
  GitHub-token values are refused with where the secret belongs (gh's own auth). A malformed file names its path and line. `stack config show [--json]` prints each setting,
  its value and its origin. A copied binary plus the config folder behaves the same on another machine.
- **Template.** The PR template is compiled in; `template = "<file>"` in stack.toml names a file in the config folder
  instead. The repo's `.github` template is never read. Required sections (`<!-- required -->`) must be filled in a
  topic's first commit, or `check` refuses it.
- **Descriptions.** Every PR body ends with the stack table (the whole chain, this PR marked), refreshed on every
  upload. A body edited on GitHub is kept with a warning (`--overwrite-body` or `overwrite_body = true` replaces it).
- **History.** Each PR gets one history comment (`<!-- stack:history v1 -->`), edited in place, with a row per new
  head; a row whose base moved names `stack diff <topic> <a> <b>`.
- **Revision pins.** Each upload that moves a topic pushes `refs/stack/<prefix>/<topic>/r<N>` and `r<N>-base`
  (outside refs/heads), so old revisions survive the force-push. `pin_revisions` (default on) and `keep_revisions`
  (default 0, keep all) in stack.toml. `stack diff <topic> [<a> <b>]` fetches the pins and runs `git range-diff`, so
  base changes stay out; the default is the last two revisions.
- **Status.** `stack status [--json]` draws the chain as a tree from PR bases (parent, depth) and shows a stopped
  restack.
- **Restack.** `stack restack edit <topic|sha>` stops at a topic's last commit (or a commit) for an amend; `continue`,
  `skip`, `abort` drive it; `--step` stops after each later topic and `--exec <cmd>` runs a check after each (exit 5
  when it fails). State is in the git dir (`stack/restack.json`), so a fresh process resumes it; `upload` refuses while
  one is stopped. A conflict names the topic, commit and files.
- **Errors.** Every error ends with a `Next:` line naming the command to run. Exit codes: 0 ok, 1 git/gh, 2 usage or
  config, 3 conflict, 4 GitHub refused, 5 restack `--exec` failed.
- When `gh stack` is installed (`native_stack`, default on), chains are also linked as native stacked PRs; a refusal is
  a note, not a failure.
- **Reverse:** set `prefix`, `template`, `pin_revisions = false`, `native_stack = false` in stack.toml; delete pins
  with `git push <remote> --delete refs/stack/...`; drop the body table in `upload::write_stack_tables`.

## D411: Every linked machine gets `stack` and the host's stack.toml (STACK manager, owner ask, 2026-11-16). Provisional
- **Binary.** `stack` is a fourth installed file (`InstalledFile::Stack`) in the tools bundle (`:tools`, `:tools_mac`)
  and in the app's Contents/Helpers. It installs, stages as `.stack.next` and upgrades like humatad2, holder and
  humata, by checksum; a newer one comes with the release's VIP Inbox Migrate (no mixed versions). The Linux builds
  are static musl, cross-built on the Mac (`//tools/stack:stack_linux`; `static_linux_test` checks for no interpreter).
- **Config.** The host's `~/.humata2/config/stack.toml` (next to the daemon's state folder) goes to the machine's
  `~/.humata2/config/stack.toml` at setup and at every link-up when its sha256 differs; the far end checks size and
  sha256 before the rename. No host file: nothing is sent and the machine's copy is left alone.
- **Secrets.** Before sending, top-level keys whose name has token, secret, password, passwd, credential, auth,
  api_key, apikey or private_key, and values that start like a GitHub token, are dropped and logged by key name only.
  Keys inside a `[table]` are not filtered (stack.toml has none); `auth` also drops a key like `author`.
- **Open:** interactive-login machines get the binary but their link-up push only logs a failure; removing a machine
  leaves its `config/stack.toml`.
- **Reverse:** drop `InstalledFile::Stack` and the `stack` attr of `tools_bundle`, and the two push hooks
  (`Setup` `Pending::Config`, `machine_service/stack_settings.rs`).

## D412: The shared stack model gains `RepoStack` for PR review (STACK manager, Opus advisor, 2026-11-16; D300, PR A10). Provisional
`//tools/stack/graph` keeps `Stack` (every PR, for the CLI's tables) and adds `RepoStack`, D300's model under that name:
the one tree that holds a PR, bottom first, with `repo()`, `trunk()`, `source()` (`Native` or `BaseChain`),
`github_number()`, `entries()`, `parent_of(n)` and `children_of(n)` (lowest number first).
- **Trunk** is not an input: it is the bottom PR's base, so it can't disagree with the PRs.
- **Forks.** `Pr` gains `from_fork`; a PR whose head is in a fork is never a parent, in `Stack` too.
- **Loops.** `RepoStack::from_base_chain` gives no stack for a PR on or under a base loop, or a PR alone; the CLI's
  `Stack` still breaks a loop at the lowest number, because its tables list every PR.
- **Native.** `RepoStack::native(repo, number, members, prs)` orders the members by their bases (the base chain is the
  truth, whatever order GitHub lists them in) and is None unless they form one linear chain; the daemon then falls
  back to the base chain.
- **Visibility** stays `//tools/stack:__pkg__`; the PR lane adds its exact daemon package when it first depends on it.
- **Reverse:** drop `RepoStack`/`Source` and `Pr.from_fork`; nothing outside `tools/stack` uses them yet.

## D420: ⌥← and ⌥→ move by word, and a Settings switch makes Option act as Alt (TERM manager pick, 2026-11-16; D73 Option as Alt, README gap 2, micro-03). Provisional
⌥← and ⌥→ always send Esc b and Esc f, and ⌥⌫ always sends Esc DEL, so moving and deleting by word works out of the box. Every other ⌥ key types the character the keyboard layout gives, such as `#` on ⌥3 on a UK keyboard. Settings › Keyboard › **Option acts as Alt** (off by default) sends Esc plus the key for every ⌥ key instead, through `set_option_as_alt`, for people whose shell or editor binds ⌥ letters. It applies to every terminal at once, on desktop and in the browser. Off by default because turning it on stops some layouts from typing characters that need ⌥, and the switch's note says so.
- **Reverse:** on by default; or drop the fixed word keys and leave everything to the switch.

## D421: ⌘W closes the selected tab, and ⇧⌘↩ toggles Terminal full screen (TERM manager pick, 2026-11-16; NV-37, D73 tabs, README gap 3, micro-23). Provisional
Not built. ⌘W (Ctrl+Shift+W off a Mac) sends `SessionAction::CloseTab` for the focused group's selected tab, the same as its ✕. It does nothing on the agent's terminal, which never closes from the tab row, and on a shell tab D423 decides. ⇧⌘↩ (Ctrl+Shift+Enter) toggles NV-37's Terminal full screen, the same as the header button. Both are bound in the app's keymap, so they work while the terminal has focus, and the Shortcuts sheet lists them. In a browser, ⌘W closes the browser tab before the page sees it, so on the web the palette's Close tab command is the way.
- **Reverse:** unbind both; the ✕ and the header button stay the only ways.

## D422: ⌘T opens a shell tab, and ⌃Tab and ⌃⇧Tab cycle the focused group's tabs (TERM manager pick, 2026-11-16; D73 tabs, README gap 4, micro-23). Provisional
Not built. ⌘T (Ctrl+Shift+T off a Mac) does what **+** › zsh does in the focused group, starting in the folder D338 records. ⌃Tab and ⌃⇧Tab select the next and previous tab of the focused group, file tabs included, and wrap around. The app binds them in its own keymap, beside its other shortcuts. `register_host_shortcuts` stays for other hosts of the reusable terminal, and the app does not call it, so a key never has two bindings. Browsers keep ⌘T and ⌃Tab for themselves, so on the web the palette's New shell tab and Next tab commands do the same.
- **Reverse:** call `register_host_shortcuts` from the desktop and handle its actions, or leave the keys unbound.

## D423: A shell tab closes with its ✕, and closing it ends its shell (TERM manager pick, 2026-11-16; D73 tabs, README gap 5, micro-23). Provisional
Not built. Every shell tab the user opened gets a ✕, and ⌘W works on it. Closing it stops its shell through the daemon and removes the tab on every screen showing the session. When a program other than the shell is running in it, a prompt asks first and names the program, because that program is stopped too. The agent's terminal still has no ✕: stopping the agent is in the session menu. The other D73 rule, that closing a view never stops a process, still holds for views. Closing the tab is what ends the shell; closing a window or a second view of the tab leaves it running.
- **Reverse:** the ✕ only hides the tab and the shell keeps running; **+** lists hidden shells to show again.

## D424: A bell flashes its tab, or marks a tab or session that is not on screen; it is silent unless a setting turns the sound on (TERM manager pick, 2026-11-16; README gap 8, micro-04). Provisional
Not built. `PaneEvent::Bell` in the selected tab of a session on screen flashes that tab for 150 ms. In a tab that is not selected, it puts a dot on the tab until the tab is selected. In a session that is not on screen, it puts the dot on the session's sidebar row too. Bells within 500 ms of the last one count as one, so a program that rings in a loop doesn't flicker. Settings › **Bell sound** (off by default) also plays the system alert sound on desktop. In the browser, the sound plays only after the page has had a click. A bell never makes an Inbox message.
- **Reverse:** sound on by default; or no dots, only the flash.

## D350: Prepared views in the app are the daemon's, by id (VIEWS manager pick, 2026-11-16; VW-2–VW-10, D289(h)). Provisional
- **Pick:** the app keeps no decisions of its own: every answer (Open it, Later, Not now, Revert, Keep, close) is one SetViewState, shown at once and put back if refused; views arrive by the event feed and a list when the app loads (every session's offers) and when the person comes back to a session (its whole Journal, Not now included, since the user's no-session list leaves Not now out). Later hides an offer for this visit; it shows again on return or after a restart. The saved `views` decisions by title are dropped (old files still load). Revert puts back route, trail, the session's tabs and panel, full screen, side-panel fold, board scope and per-repo diff targets; comments are never touched. A review plan opens the Diff panel with its per-repo targets.
- **Reverse:** `features/views/src/lib.rs`, `features/app/src/views.rs`, `ui/src/view_tab.rs`, `Clients.views`.

## D351: Prepared views stack; Keep and Close resume the one under (VIEWS manager pick, 2026-11-16; VW-9, D350). Provisional
- **Pick:** each opened view pushes its own Revert snapshot; reopening a view already in the stack pushes nothing. Revert pops the top and shows the view under it, so a second Revert goes back to the page. Keep and Close drop only the top snapshot and show the view under it; Keep leaves the tab open. A walkthrough step opens its file beside the view and Next replaces only that file; a step in a repo the session doesn't hold opens nothing. A board view scoped to a session opens that session's board; one for a session the app doesn't know shows as a summary.
- **Reverse:** `features/app/src/views.rs`, `ViewsAction::Resume`, `ViewContent::SessionBoard`.

## D352: A machine's views reach the host only when offered, never while away (VIEWS manager pick, 2026-11-16; VW-12, AW-7, D226, D227). Provisional
- **Pick:** only a draft's move to Waiting on a machine journals `view.offered` (must keep, before the agent hears back); drafts, revisions and key-only MarkReady calls are not journaled. PrepareView and MarkReady stay refused while the host is away (AW-7, D227 unchanged). The host owns an offered view's state from then on: replay inserts once and never overwrites an answer. D226 pick 5's replay list gains `view.offered`.
- **Reverse:** `review/views.rs` mark_draft_ready, `away/journal.rs` record_view, the host's `view.` replay arm, `apply_machine_journal_view`.

## D360: A stop's time reads as clock text, "14:02" in the fakes and frames (STOP manager pick, 2026-11-16; D260, frames 73–76). Provisional
The page banner and the Hosts row say when the sessions were stopped as the local clock time of the stop record (`HH:MM`). The fakes and frames use a fixed "14:02" so captures stay stable.
- **Reverse:** show a relative time ("12 min ago") and update frames 73–76.

## D361: A stop with no recorded stopper reads "by you" (STOP manager pick, 2026-11-16; D264). Provisional
A stop record or a host copy whose `by` is missing (an older record, or a relayed stop that lost it) reads as the user's own stop. Only the user and allowed rules can stop a machine, so this is the likeliest case.
- **Reverse:** add an "unknown" stopper and its banner wording.

## D362: A stop that hasn't reached its machine leaves the sessions' statuses as they are (STOP manager pick, 2026-11-16; D262, frame 76). Provisional
Until the box takes the stop, its sessions keep their last known status (usually offline). Frame 76's banner says they may still be running. They read Stopped only once the box reports the sweep.
- **Reverse:** mark them Stopped at once on the host, which says something the host can't know.

## D363: The Stop and Start confirms centre on the page area, right of the sidebar (STOP manager pick, 2026-11-16; frames 72, 75). Provisional
They read as the machine page's own dialogs, as frame 71's delete dialog does.
- **Reverse:** centre them on the window.

## D364: A stop or start cut by power is proven on the host; the box's own paths are crash-cut (STOP manager pick, 2026-11-16; exit criterion 5, D243). Provisional
`stop_sessions_test` cuts the power at each of the 7 stop and 4 start points, through humata2_durable's simulation, with the holders cut and every program under them killed. A box runs the same stop and start code. Its own paths (`humatad2 stop`, the host copy, the relay) are crash-cut in `stop_box_test`, not power-cut. The simulation keeps unsynced writes in the process that made them, so a write a stop leaves unsynced before it restarts itself counts as kept. The stop syncs its record, rename and folder before each point, so nothing is known to depend on that.
- **Reverse:** add a Stop operation to the PER-28 rig (`tests/common/power_cut.rs`) and power-cut the box too.

## D370: The host posts "Migrate the daemon on This Mac" when a verified release is staged in its bin (UPGRADE manager pick, 2026-11-16; U7, force-upgrade). Provisional
The host daemon watches `<bin>` (notify, non-recursive), and looks once at ready before it serves. A staged `.bundle.json.next` that differs from its own bundle and verifies posts a Daemon item for its own machine; `to` comes from the staged manifest (version empty). Migrate runs the existing local Runner; arrival at the new bundle ends it Approved. Unstaging withdraws the item as resolved, unless it is Migrating. A daemon with no bundle posts nothing (D290 l). Only a host runs the watch, so no machine posts its own items twice.
- **Reverse:** drop `upgrade/local.rs` and its start in `serve.rs`.

## D371: The CLI and host tools are their own Migrate kinds, local and remote; a Daemon item folds them in (UPGRADE manager pick, 2026-11-16; U7). Provisional
The CLI is `<bin>/humata` and host tools are `<bin>/humata2_holder`. A file that is present and whose sha differs from the bundle posts a Cli or HostTools item; a missing file never posts. Remote machines report what is installed in `BuildInfo.installed` (build.proto 6), compared with their own `files`. Migrate for these is `Upgrade` with `to_bundle` equal to the daemon's own bundle: it renames only the differing tools from verified `.next` copies, with no record and no exec (FailedPrecondition when none are staged). The item ends Approved when the carry returns. While a Daemon item is pending, tool mismatches become lines of it and a pending Cli or HostTools item is withdrawn.
- **Reverse:** drop `upgrade/tools.rs`, the tools-only arm of `run.rs` and `BuildInfo.installed`.

## D372: A schema change is a line of the daemon item, not its own item (UPGRADE manager pick, 2026-11-16; U7). Provisional, owner question
When both store schemas are known and differ, the Daemon item's changes list the store line. There is no standalone Store item: a store older than its binary keeps migrating at open, with SAFE's copy before it. Owner question: should a store found older on its own post its own item?
- **Reverse:** post a Store item from the store open path.

## D373: "Migrate the holders" restarts idle agent sessions onto the current holder; shells stay (UPGRADE manager pick, 2026-11-16; U6, PER-5, PER-6). Provisional, owner question
Each holder spawn writes `<state>/holder_builds/<session>.sha` (shells: `<tab>.shell.sha`) before the holder starts. When the daemon's own `--holder` file matches the bundle, agent sessions whose record differs, or that have none, are listed in one Holders item for this machine; shells on old holders are listed as keeping running and never hold the item open. Plain terminal sessions count as shells. Stopped and exited agents drop out. Migrate restarts each listed agent at its next Idle or ProbablyIdle moment (AgentRecord Resuming, `restart_agent` with Why::Moved, same session and conversation); an agent with no turn tracking is never auto-restarted. The item has no deadline and ends Approved when none waits; a restart error marks it Failed so Migrate retries. Remote holders are not covered yet. Owner question: does ProbablyIdle count as idle?
- **Reverse:** drop `upgrade/holders.rs` and the `HolderBuilds` hook in revive.

## D374: The app posts its own item for an older local daemon (UPGRADE manager pick, 2026-11-16; U7, UPGRADE exit line 1). Provisional
When install staged a release and the local daemon's ServerInfo has no build, the app shows "Migrate the daemon on This Mac", held by the app, not the daemon. Migrate checks the staged release is still the one shown, then `humata2_install::migrate_old_daemon` verifies every staged file, renames them with the bundle last, and kickstarts the daemon once. Known gap: the app has no reconnect, so the item stays Migrating until Humata is reopened.
- **Reverse:** print the staged-release message again in `desktop/src/main.rs`.

## D375: A Migrate card has one verb and skips bulk answers (UPGRADE manager pick, 2026-11-16; U7, D281k). Provisional
A Migrate item takes only Migrate: Answer, Always, Choose and reply do nothing on it; Allow all/Deny all skip it and never ask on its account. Approving returns at once and the item shows Migrating; Failed shows the error and Migrate again. Machine pages are read-only while a Daemon item waits (new session and shell are refused with `MigrationNeeded`, pointing at the item). `DaemonError::MigrationNeeded` carries no source because `DaemonError` is Clone and Eq. Until the session feature carries a structured refusal, the session notice shows while its machine has an unanswered Daemon or Store item.
- **Reverse:** drop the Migrate arms in `features/inbox` and `ui/src/request_card.rs`.

## D376: After migrating an old local daemon, the app waits for it and reconnects (UPGRADE manager pick, 2026-11-16; batch 6, completes D374). Provisional
Once `migrate_old_daemon` returns, the Inbox waits for the local daemon to answer at the staged bundle (`LocalDaemon::back_at`: notify on the socket's folder, one connect per event) against a 60 s app-clock bound. Back in time: the item ends Migrated and the app reloads and follows events, as after any daemon restart. Late, or back at the old bundle: the item is Failed with what to do (kickstart the daemon, or reopen Humata) and Migrate again on offer. `OldLocalDaemon(None)` ends a Migrating item Migrated instead of withdrawing it.
- **Reverse:** drop `back_at` and the two actions; the item stays Migrating until Humata is reopened.

## D377: A session keeps a structured "Migrate first" refusal (UPGRADE manager pick, 2026-11-16; batch 6, replaces D375's inference). Provisional
`SessionState.refused` is set when send, resume or opening a terminal is refused with `MigrationNeeded`, and cleared by the next success or when the named item (or, unnamed, the same thing on the same machine) shows Approved. The session notice draws from it only. Typing and writable attach set it too (amended 2026-11-16, STOP+UPGRADE batch 2): the live link turns a `MigrationNeeded` refusal on attach open or mid-stream into `TerminalNotice::Refused`, decoded as send's is; the terminal view forwards it as `SessionAction::Typed(Err)`, and a written line or a writable-attach confirm sends `Typed(Ok)`, which clears it. Other typing failures stay toasts in the terminal view; the keystroke gate stays the daemon's.
- **Reverse:** draw the notice from the machine page's migrations again; for typing, drop `SessionAction::Typed`, `TerminalNotice::Refused` and `Pump::refuse`.

## D378: A machine reports its old holders to the host, which posts and carries its Holders item (UPGRADE manager pick, 2026-11-16; batch 6, U6 remote). Provisional
A machine-role daemon runs the holder-record detection on itself and pushes `MachineToHost.holders` (peer.proto 12) after each Resume and on each change. The host posts that machine's Holders item from it. Migrate sends `Upgrade{thing = HOLDERS}` (daemon.proto `UpgradeRequest.thing = 3`), which starts the machine's restart loop and is idempotent. A dropped link mid-migrate leaves the item Migrating; on re-link the host asks again while sessions wait, and the item ends Approved when a report lists none. A failed restart on a machine is logged there and retried on the next link; the host doesn't show it Failed.
- **Reverse:** drop the report from the uplink and `thing` from `UpgradeRequest`.

## D379: A Migrate item carries its steps, per-session holder notes and a folded change list (UPGRADE manager pick, 2026-11-16; batch 6, frames 78, 79, 83). Provisional
`MigratePreview.steps` (10) and `holders` (11). Steps are set at Migrate and saved with the item, so they survive the daemon's exec; each advance is one pushed update. This Mac's daemon: Check files, Copy the store, Swap files, Restart, Back at the new release (the store copy is made by the new daemon at its first start). A machine's daemon: Copy files over SSH, Swap files, Restart, Link back. The CLI and host tools: Swap files. Holders have none. Arrival marks every step done; a failure marks the step under way as failed. Holder rows read restarts when idle, restarting, no idle moment (no turn tracking), or stays until closed (shells). A card with more than 3 change lines starts folded to 3 and "N more"; the fold is per item and survives updates.
- **Reverse:** drop the two fields and draw the batch 5 card.

## D400: Off a Mac, the desktop keeps its state in the XDG data folder (LINUX manager pick, 2026-11-16). Provisional
`$XDG_DATA_HOME/humata2` (live) and `…/humata2-fakes`, or `~/.local/share/…` when `XDG_DATA_HOME` is unset, empty or relative; macOS keeps `~/Library/Application Support/Humata 2` and `Humata 2 (fakes)`. Lowercase, no spaces, as Linux folders usually are; nobody runs the Linux app yet, so nothing moves.
- **Reverse:** `default_data_dir` in `desktop/src/main.rs` returns the macOS folder names under `~/.local/share`.

## D401: The Linux batch builds on the Mac and only runs tests in the VM (LINUX manager pick, 2026-11-16; refines D18). Provisional
`testing/linux/run.sh` runs `bazel test --config=linux_arm64 --run_under=testing/linux/in_vm.sh --strategy=TestRunner=local`: each test's runfiles stream into the VM under `/v` at their Mac paths, run there with the test's full env and args, and its outputs come back. Two tests at a time (2 CPUs), and Bazel caches passes, so a rerun only runs what changed. `in_vm.sh` isn't a Bazel input, so after changing it rerun with `--nocache_test_results`.
- **Reverse:** go back to copying each binary and running it bare (the old `run.sh`).

## D402: GPUI suites on Linux build for glibc against a pinned sysroot (LINUX manager pick, 2026-11-16). Provisional
gpui-kit turns on GPUI's X11 and Wayland backends on Linux, which link libxkbcommon, libxkbcommon-x11, fontconfig and freetype, so nothing that links GPUI builds for static musl. Those suites (`testing/linux/batch_targets_gui.txt`) build with `--config=linux_*_gnu` against a sysroot of pinned, hashed Ubuntu packages for those libraries (a MODULE.bazel change, main's). Daemon and CLI suites stay static musl.
- **Reverse:** build GUI suites inside the VM instead (slow on 2 CPUs), or patch gpui-kit's platform features.

## D403: The Linux launch smoke test runs on lavapipe under Xvfb and headless Weston (LINUX manager pick, 2026-11-16; method for D129). Provisional
Mesa's lavapipe gives the VM a software Vulkan device for GPUI's wgpu renderer. X11: `Xvfb` with `Xft.dpi: 192`; Wayland: `weston --backend=headless --scale=2`. Frame 01 must be non-blank, `App::on_missing_glyphs` must report nothing, and the window's scale factor must be 2.
- **Reverse:** pick another software renderer (llvmpipe GL) or give the VM a virtual GPU.

## D404: Off a Mac, the UI's Menlo and system-font families map to bundled fonts (LINUX manager pick, 2026-11-16; reading of D129's "bundled fonts"). Provisional
The UI names `Menlo` and `.SystemUIFont`, which Linux lacks; GPUI would fall back to whatever the machine has, or panic with no fonts at all. Off macOS the desktop registers the Inter and JetBrains Mono files `terminal/client/assets` already ships through `engine::Resources::font`, and the two family names map to them. Not built yet (LINUX task T5).
- **Reverse:** rely on the system's fonts (fontconfig fallback) and install fonts in the VM only.

## D405: Linux containment is Landlock, and fails closed where Landlock can't hold a rule (LINUX manager pick, 2026-11-16; for SEC-6/D128 on Linux). Provisional
`daemon/contain` names Landlock plus a network namespace as LINUX's. Landlock only allows paths, so "hidden inside readable" becomes an allow list of every readable root minus the hidden ones; ABI 4 (Linux 6.7+, Ubuntu 24.04) limits TCP connects to 443; DNS is UDP, which Landlock can't limit; and a path-named unix socket can't be limited to the daemon's alone. Where the kernel's ABI is too old for a rule, `wrap` refuses (D217) rather than run looser. Owner question: is "TCP 443 and DNS, plus any local unix socket the agent can see" acceptable on Linux, or must Linux add a network namespace with a forwarded socket before contained agents run there?
- **Reverse:** keep `ContainError::Unsupported` on Linux (contained agents are macOS-only) and drop SEC-6 from the Linux batch.

## D406: The Keychain stays fake on every platform for LINUX (LINUX manager pick, 2026-11-16). Provisional
`clients/src/keychain.rs` has only `FakeKeychain`, the live desktop included, so it is a gap on macOS too, not a Linux one; a Secret Service backend is a new feature, out of LINUX's scope.
- **Reverse:** add a Secret Service (Linux) and Keychain (macOS) backend as its own milestone.

## D407: The Linux desktop installs its local daemon as a systemd user service (LINUX manager pick, 2026-11-16; PER-18 for This Machine). Provisional
`desktop/install` writes only a launchd agent. On Linux it writes `~/.config/systemd/user/humatad2.service` from the same unit text `daemon/machines/src/service.rs` writes on remote machines, runs `systemctl --user enable --now`, and asks for `loginctl enable-linger` as PER-18 says. Not built yet (LINUX task T4).
- **Reverse:** start the daemon from the app on Linux (no service), losing reboot survival.

## D425: The mouse is the history's whenever no program can hear it; ⇧ always keeps the wheel for the history (TERM manager pick, 2026-11-16; README bugs 8 and 9, micro-14). Provisional
While a program takes the mouse (htop, vim with `mouse=a`), a wheel with ⇧ held scrolls the history and sends nothing, the same as ⇧ keeps a drag for selecting. On a Mac a ⇧ mouse wheel arrives as a sideways scroll, and it scrolls the history too. When nobody can hear the mouse because the tab's program ended, the link dropped or the tab is read-only, the wheel scrolls the history and a drag selects, even if the program left mouse reporting on. Not built: a program that crashes with mouse reporting on and leaves its shell running still gets reports at the shell. The pane can't tell that the program is gone. Fixing that needs the holder to notice the shell is back in front and send the reset itself.
- **Reverse:** ⇧ wheel reaches the program like any other wheel; or keep sending reports after the program ends.

## D426: A retried Type asks the tab's holder whether the first call's text arrived (TERM manager pick, 2026-11-16; follows D337, KeyState/KeyedInput). Provisional
A keyed Type now types through the holder's keyed input, which buffers the whole text, writes it at once and remembers the key. Before typing, the daemon notes that holder's instance with the pending key (store step 29). A retry that finds the key pending asks the same holder. If it typed the text, the retry answers typed and types nothing. If it didn't, the retry types it now, and the holder's own key check means two racing calls still type it once. The old "may have been typed" answer remains only when nobody can tell: the holder was started again (new instance), is from a build without keyed input (it gets plain typing), didn't answer, or its instance couldn't be saved. Known limit: a holder remembers its last 256 keys, so a retry after more than 256 other keyed inputs into the same tab could type the text again.
- **Reverse:** keep plain typing and the "may have been typed" answer for every unfinished key (D337 as it was).

## D435: The restart and fork live tests wait on the agent's writes, not on a new Output event (WIRE-A manager pick, 2026-11-16; follows D399). Provisional
`live_flows/restart.rs` (another session; Start fresh) and `live_flows/fork_kinds.rs` (the second turn) check the conversation, then wait on `World::wrote()`, WIRE-B's agent-write fifo, until the next write or the deadline, as `running_session` already does. They no longer sleep between reads. The approved Output event on `events.proto` was not added: `wrote()` already wakes the test on each write, and nothing in the app would read a raw-output topic. A draft of it (`TOPIC_OUTPUT`, `SessionOutput`, joined in the agent watch) exists in the lane's scratch work but is not shipped.
- **Reverse:** add the Output topic and payload and have these tests wait on it.

## D436: A live test that reboots the daemon follows its events, then waits for its restart report (WIRE-A manager pick, 2026-11-16; PER-29). Provisional
`reboot_and_settle` in `live_flows/restart.rs` opens the daemon's event feed first, looks at the workspace once, and if no restart is known yet waits for the client's `Event::Restart`. The live client reads the report on the first daemon event after it is published. Following first and looking second means a report made between the two is still heard. The test no longer reloads every 100 ms.
- **Reverse:** reload the workspace in a loop until it lists a restart.

## D437: A box session's pin and VIP mark stay on the host (WIRE-A manager pick, 2026-11-16; NV-24). Provisional
Marks are the person's view, and the host is where the person is: `SetMarks` is host only and the relay lets a box send only marks it reads, not pins (CLI.md, relay allowlist). The host fills `pinned` and `vip` from its own marks on every path that shows a session (GetSession, ListSessions, Session events). A box's report lays only its status, exit code, pointers and unread over the record, so it never takes a mark away. Keeping a copy per machine and forwarding SetMarks to the owning box would give two copies to keep in step for no reader on the box. `daemon/humatad/tests/read_marks_remote.rs` proves the marks survive the box's report.
- **Reverse:** keep a per-machine copy and forward SetMarks to the owning machine, with a field on the box's report (an api/proto SEAM).

## D430: An archived session's folder is freed only once its undo window closes, also after a restart (daemon triage, 2026-11-16). Provisional
Startup schedules each free at `free_after_unix_ms` rather than freeing at once. A channel restored from the trash gives its sessions a fresh undo window from the restore, and one archived before the delete keeps the later of its own and the restore's. Freeing early deletes the person's folders.
- **Reverse:** free every archived session's folder at startup, and let a restored channel's sessions keep the window they had.

## D431: Only the host closes a channel Desk session whose channel is gone (daemon triage, 2026-11-16; CD-5, AWAY). Provisional
Under AWAY a machine runs with the host closed, but its store can't know whether a channel is gone, so a machine's daemon skips CD-5's watch. The host closes those sessions when it comes back. A restore forwarded to a machine asks the channel's TrashRestore rule on the host before CD-4 and CD-5's checks, so a caller who may not restore there learns nothing of the channel or the target.
- **Reverse:** run CD-5 on every machine from its own store, and judge the restore's preconditions before its permission.

## D432: The host journals each session start it asks another machine for until the machine answers, and a restart report's restart.json is written last (daemon triage, 2026-11-16; PER-29, D150, D241). Provisional
Before saving a remote session's record, the host writes `<state>/carry/remote_archive/start/<session>.json` with the start and its operation. The file goes when the machine answers with the session or refuses the start, also when that answer comes on a later link. At start, each file left over brings its operation back, shows as pending in the restart report, and asks the machine again under the same id once it links. The machine answers with the session it kept, or starts it. If the host no longer has a live record, the file is dropped. If the machine refuses, the host's record goes too. A power cut on either side so never leaves a host record that nothing runs. The restart report writes `operations.jsonl` before `restart.json`, because a reader takes `restart.json` as the sign that the report is whole.
- **Reverse:** keep the lost answer's retry in memory only, and write the operations after `restart.json`.

## D450: A doc's versions are git commits in its Desk folder; restore adds a version (owner, 2026-11-16; DOCV). Signed off
Owner picks, verbatim in `workflow/milestones/DOCV.md`: a version is a git commit in the doc's folder; the agent commits with plain git when it wants and the user with **Save version**; there is no automatic snapshot per turn; restoring v3 saves it as a new newest version and v4 and later stay. "I never want to be picking folders": the New desk session dialog has no folder choice and no path, and every Desk folder is made by Humata under the machine's root. Desks are scoped global and per channel (CD-1). SPEC DK-2, DK-3, DK-14, DV-1–DV-13.
- **Reverse:** owner's call only.

## D451: Every Desk folder is a git repo, kept once it holds a version (DOCV manager pick, 2026-11-16; Opus taste advisor). Provisional
The dialog asks no storage question. Every Desk folder Humata makes is a git repo; it is kept for good once its repo has a commit (Save version or the agent's `git commit`) or the user clicks Keep; otherwise it goes 7 days after close, as scratch folders did. What the user does decides, so flow 1's throwaway look cleans itself up and flow 4's doc can't vanish by surprise. Closing an unkept folder that holds files asks once, with **Save version and close** (DV-4). The advisor also suggested auto-saving a version on close; not taken, because the owner ruled out automatic snapshots. Sessions started in a picked folder before this keep working there; there is no way to start a new one. Recommendation: keep.
- **Reverse:** (A) the dialog asks "A doc to keep" or "A quick look"; or (B) every Desk folder is kept and only Clean up… removes them.

## D452: A channel Desk session's doc folder stays in place and lives as long as the channel (DOCV manager pick, 2026-11-16; with ROAM's CD-1, CD-4). Provisional
CD-1 makes channel Desk sessions short-lived, and CD-4 trashes their folder at once on Close. A channel Desk folder is a repo too (DV-1); once it holds a version it is kept: Close stops the session but leaves the folder at `<root>/<channel>/desk/<name>`, listed on the channel's Desk tab under Closed and Documents, moved with the channel's machines (MV-16, MV-17), and trashed only when the user deletes its session or the channel. Retention rules don't touch it. A folder with no version is trashed at once, as CD-4 says. So the managerial sessions stay short-lived while a channel's spec belongs to the channel. Recommendation: keep.
- **Reverse:** on Close, a channel Desk session's Markdown moves into `shared/` (one repo for the channel's docs) and the folder goes to trash as CD-4 has it.

## D453: A version is a commit that changes the file; who made it comes from a trailer (DOCV manager pick, 2026-11-16). Provisional
History is per file (`git log -- <path>`), numbered from the oldest; lines added and removed come from `--numstat` for that path. Humata's own commits carry `Humata-Saved-By: you` and show as **You**; any other commit shows as the session's agent, whatever author it names. The repo's local `.git/config` holds an identity named for the session's agent at a local address, so a plain `git commit` works on a machine with no global identity; it never names the user.
- **Reverse:** tell agent and user apart by author name, and leave the identity to the user's global config.

## D454: Humata writes only its own repo's local config; its own commits run without hooks or signing (DOCV manager pick, 2026-11-16). Provisional
`git init` with no commit, so a fresh folder is not kept (D451). Each clone is listed in `.git/info/exclude` when it is added, so the folder's repo never tracks clones or makes gitlinks. Save version and restore run `git -c core.hooksPath=/dev/null -c commit.gpgsign=false commit …` so a user's global hooks or signing key never prompt or fail; nothing is written to the global or system config, and no clone or picked folder is ever committed to by Humata.
- **Reverse:** let Humata's commits use the user's hooks and signing.

## D455: Restore first saves unsaved changes, then writes only that file (DOCV manager pick, 2026-11-16; follows D450). Provisional
Restoring v3 with unsaved changes saves them as "Saved before restoring v3", then writes v3's text to the one file and saves "Restored v3". Nothing is lost, and other files in the folder are left as they are. Restore is the user's change, so comments re-anchor (SM-7) and none is addressed (SM-16).
- **Reverse:** restore the whole folder to that commit's tree; or refuse while there are unsaved changes.

## D456: Comments on an old version come from a record taken at each commit (DOCV manager pick, 2026-11-16). Provisional
Comments live in the store, not in the file, so "as they were" needs a record. When a Desk folder's repo gains a commit, the daemon stores, per changed file, the session's comments on it (drafts included, archived left out), keyed by the commit. An old version shows that record read-only; a version made before the record existed says comments weren't recorded. The daemon notices commits by watching the repo's HEAD, so the agent's plain `git commit` is recorded too.
- **Reverse:** show the current comments re-anchored onto the old text (no record), or show no comments on old versions.

## D457: The agent reads versions with two MCP tools and saves only with git (DOCV manager pick, 2026-11-16). Provisional
`doc_versions_list` and `doc_version_read`, no rule for its own session's folder (as `comments_list`, D21), and the file-read rules for another session or machine. No save or restore tool: the owner said the agent saves with git, and a tool would duplicate it; the agent can still restore by writing the old text and committing. No new access switch, since both are reads under existing rules.
- **Reverse:** add `doc_version_save` and `doc_version_restore` tools, each with its own rule.
- 2026-11-17 (Fable ruled during the gate-30 fixes): a `desk_peer` access clause had let one Desk session read another's folder and versions. That went against D182.3, so it was reverted in `access/src/policy.rs`. Desk sessions have no channel-mates.

## D458: Desk folders made before DOCV get a repo through a Migrate item (DOCV manager pick, 2026-11-16; UPGRADE force-upgrade rule). Provisional
An open or kept Desk folder Humata made before DOCV has no repo. On upgrade, one VIP Inbox "Migrate Desk folders" item runs `git init` as DV-1 in each, with no commit, so whether it is kept doesn't change. Folders the user picked are never touched.
- **Reverse:** leave old folders without history (no History strip on their files).

## D460: The user picks repos, machines, channels and sessions, never folders (owner, 2026-11-16; NOPATH). Signed off
Owner, word for word: "I never want to be picking folders. The app should be opinionated about how it handles for me. I should be picking higher level things like repos I need, machines I need. … If I want to move files between machines it should show me potential machines to move it to and not where on those machines UNLESS I ASK FOR IT TO MOVE IT SOME PLACE SPECIFICALLY. When I ask to sync sessions it should sync it for me to where it needs. When I ask to make a spec it should pick a folder to make it for me. These things should be somewhat configurable as like the 'roots' but once those roots are set they should be opinionated." Rules PL-1 to PL-13; FC-1, CH-18, DK-16, DK-17, SE-18, PR-21, FM-1, FM-2, FM-7, IN-10 and ST-5 follow. CH-18 loses its local-path source. Golden frames that show or ask for a path go to the owner's wave; none changes before then.

## D461: D315 is withdrawn; "Move the picked folder" and draft frame 96 are moot (NOPATH manager pick, 2026-11-16; D460, D451). Provisional
D315 asked the user to tick "Move the picked folder" before a Desk session with a picked folder could move into a channel. Under D460 nothing asks where, and since D451 no new session starts in a picked folder. A folder picked before D451 is copied to the channel place like a scratch folder, and the original stays where it was, so nothing the user owns is moved or deleted. `MoveDeskSessionToChannelRequest.move_picked_folder` is ignored and marked deprecated. Draft frame 96 is dropped from ROAM's list. Recommendation: keep.
- **Reverse:** keep D315's tick for legacy picked folders.

## D462: A machine-only send lands by its source's place; anything else lands in From <machine> (NOPATH manager pick, 2026-11-16; Opus taste advisor; PL-3, PL-4). Provisional
A channel's shared files go to that channel's shared folder on the target. A session's files go to the channel's shared folder there, under a folder named for the session, so two sessions don't mix. Home and Desk files go to `<root>/received/<source machine>/`. A machine outside the channel gets the file in its received place and the channel is not changed, since adding a machine is a bigger step. Recommendation: keep.
- **Reverse:** everything goes to received/, or sending a channel file adds the target machine to the channel.

## D463: Copy to ▸ and Move to ▸ list machines; Somewhere specific… is the explicit form (NOPATH manager pick, 2026-11-16; Opus taste advisor; PL-5, amends D165). Provisional
The menus list the channel's machines first, then the others, each with its landing place by name, then Somewhere specific…. The advisor recommended dropping the second pane, since it exists only to choose a destination. The two panes stay (D165 is the owner's) until the owner sees frame 61's redraw in the wave. Recommendation: keep the panes for now; ask the owner about one pane.
- **Reverse:** one pane, with Somewhere specific… as the only way to browse a target.

## D464: Places are shown by name: breadcrumbs, and received files as From <machine> (NOPATH manager pick, 2026-11-16; Opus taste advisor and main; PL-4, PL-11). Provisional
A pane header reads "This Mac › # ml › shared › datasets". The machine crumb lists channels, Desk, each From <machine> and Home last. Main asked that the received place be named by purpose, never by path. Recommendation: keep.
- **Reverse:** show the real path in the header.

## D465: Each machine's root is a Settings row; Add machine never asks; `host root` with no path prints it (NOPATH manager pick, 2026-11-16; Opus taste advisor; PL-1, PL-12, FC-10). Provisional
Settings › Machines shows **Humata folder** with the path muted and **Change…**, which opens a folder picker on This Mac or a field for a remote, with Use default, then FC-10's plan. The CLI `host root <machine>` prints the root, `--default` resets through the plan, and `host add --root` stays optional and is never prompted. A command without a path that silently resets a root could move channels. Recommendation: keep.
- **Reverse:** Add machine asks for the root, prefilled; `host root <m>` resets.

## D466: Paths are hidden; Reveal and Copy path sit in the row's … menu; New channel shows machines, not its folder (NOPATH manager pick, 2026-11-16; Opus taste advisor; PL-9, PL-10). Provisional
Reveal and Copy path are in the right-click and … menus only, never on hover. On a remote machine Reveal is **Open terminal here**, a terminal on that machine in that folder. The New channel dialog's "Folder: ~/Humata/<slug>/" becomes "On: This Mac, gpu-box"; the Desk folder bar's path moves to its … menu (with DOCV, DK-3). Recommendation: keep.
- **Reverse:** keep paths as muted, read-only text.

## D467: The Inbox cross-machine copy card shows what, its size and machine → machine (NOPATH manager pick, 2026-11-16; Opus taste advisor; IN-10). Provisional
For example: "fix-login wants to copy datasets/ (2.1 GB) from This Mac → gpu-box · ml shared", with Approve, Somewhere else… and Deny. A place the agent asked for shows as breadcrumbs, and real paths sit behind Details. The decision is whether these bytes go to that machine. Recommendation: keep.
- **Reverse:** keep source path → destination path.

## D468: A drop lands where it's dropped; Upload… with no folder open goes to the session's pastes (NOPATH manager pick, 2026-11-16; Opus taste advisor; PL-6, RV-35). Provisional
A drop target is an explicit ask. Uploads are usually input for the agent, and putting them in a clone would leave untracked files in git. Recommendation: keep.
- **Reverse:** Upload… goes into the open folder, clones included.

## D469: A machine-only send keeps both when the name is taken (NOPATH manager pick, 2026-11-16; Opus taste advisor; PL-13, amends D142). Provisional
The rule chose the place, so the rule settles a clash, and Keep both never loses data. A byte-equal file is not copied again. A send to Somewhere specific… keeps D142's Fail / Replace / Keep both question. Recommendation: keep.
- **Reverse:** ask every time, or default to Replace.

## D470: The repo library is the host's own SQLite file; the scope is config/repos.toml (REPOS manager pick, 2026-11-16; RL-1, RL-7). Provisional
The library changes on every use (add, last used, remove), so it lives in the host's store in a file of its own, like the pins (`daemon/marks`), WAL with every write a transaction. The scope is a setting the user edits, so it is `owners` and `include_me` in `config/repos.toml`, as the portable-config rule asks. Writing last-used times into a synced TOML would rewrite the file on every machine at every use. The Opus advisor agreed.
- **Reverse:** keep the library as `[[repo]]` tables in `repos.toml`.

## D471: What puts a repo in the library (REPOS manager pick, 2026-11-16; RL-2). Provisional
A successful add anywhere puts a repo in the library: a channel's create or Add a GitHub repo…, an approved agent proposal, Add repo to this session, the Desk's Add repo and `humata desk add`. Starting a session with a clone of it moves last used forward. Searches, ticks, cancelled sheets and failed clones add nothing, so the library holds only repos the user really used. Removing a repo from a channel keeps it in the library.
- **Reverse:** also add every repo the user ticks, or drop library entries when their last channel lets them go.

## D472: Settings offers Remove per repo and Forget all… (REPOS manager pick, 2026-11-16; RL-4). Provisional
Remove takes one repo out. Forget all… empties the library after a confirm that gives the count and says nothing is deleted on disk or on GitHub. It is the reset when moving accounts. Neither touches channels or clones. The Opus advisor recommended (a).
- **Reverse:** Remove only.

## D473: Machines get repos.toml, never a copy of the library (REPOS manager pick, 2026-11-16; RL-12, D411). Provisional
`repos.toml` goes to every machine the way `stack.toml` does (D411), secrets filtered. The library stays on the host as the one source. A machine's CLI asks the host, and while the host is away it says the host is away instead of searching a stale copy. The Opus advisor recommended this over a read-only `repo-library.toml` on each machine, because no copy means nothing stale and no mixed versions.
- **Reverse:** push a read-only `~/.humata2/config/repo-library.toml` on link up, so a box can search while the host is away.

## D474: The default scope is the listed owners plus you; with none listed it is Your repos (REPOS manager pick, 2026-11-16; RL-7, ST-21). Provisional
Keys: `owners = [...]` (GitHub's word for an org or a user) and `include_me = true`. With `owners` set, the scope is those owners plus the user's own login. gh's collaborator and org-member listing then stays out, which is the noise the owner wants scoped away. With no `owners` key, the scope is Your repos (CH-3's listing), so a fresh install works as it does today. The advisor suggested making that listing opt-in in every case. The fallback is the manager's.
- **Recommendation:** as written.
- **Reverse:** make Your repos the default always, with owners added to it, or require owners to be set.

## D475: Scope chips, + Org, All of GitHub, and Save as default only while the scope differs (REPOS manager pick, 2026-11-16; RL-8). Provisional
The chips show the scope at a glance, and opting out is one click, which is what the owner asked for ("manually opt out of that when I am searching"). A widened scope dies with the picker. Save as default shows only while the scope differs from the default, and only the user can use it. The Opus advisor recommended chips over a dropdown.
- **Reverse:** a scope dropdown beside the field.

## D476: The scope never filters the library group (REPOS manager pick, 2026-11-16; RL-6). Provisional
Library repos are the user's own references, and hiding them under a narrow scope would make them seem lost. The scope limits the GitHub group only. Within the library group, D274's match tier comes first and last used breaks ties, so a precise name beats a recent weak match. The Opus advisor recommended this.
- **Reverse:** filter the library by scope too, or order the library by last used alone.

## D477: GitHub search needs 2 characters within owners, 3 for All of GitHub, with a pause and a one-minute cache (REPOS manager pick, 2026-11-16; RL-9, D274). Provisional
`gh api search/repositories` with `q=<query> user:<owner>…` (`user:` matches orgs too) is sent after a 300 ms pause in typing. Each answer is cached for a minute per query and scope. That keeps typing under GitHub's search limit of about 30 a minute. Your repos keeps CH-3's cached listing and the local D274 match. The Opus advisor proposed 3 characters for All of GitHub, because 2-character global queries are noise.
- **Reverse:** one threshold for both, or no pause.

## D478: A failed GitHub search leaves the library answering and says what to do (REPOS manager pick, 2026-11-16; RL-10, AGENTS.md errors). Provisional
The daemon answers with the library group and one typed GitHub problem instead of failing the call. The problems are: gh missing, signed out, rate limited (with GitHub's reset time when gh prints it, and a reminder that signing in again doesn't lift it), GitHub unreachable from the host, and an unknown owner (marked on its chip). The client renders the daemon's text. The `gh_failure` split in `channel_service/gh_repos.rs` is the base.
- **Reverse:** fail the whole search on a GitHub error.

## D479: Agents may widen one search but never save a scope or remove a repo (REPOS manager pick, 2026-11-16; RL-11, CH-26, D278). Provisional
Agents often need a repo outside the default scope, and refusing it would only send them to the owner. Saving the scope, Remove and Forget change the owner's lasting state, so only the user can do them. If agents ever need these, they get Agent-access switches, off by default. `repo_search` is a global read like SearchGitHubRepos (D278). The Opus advisor recommended this.
- **Reverse:** limit agents to the default scope.

## D480: A terminal tab keeps the program's title and folder apart from its name (TERM manager pick, 2026-11-16). Decided (owner via Fable, 2026-11-16)
The holder already reads the title a program sets (OSC 0 and 2) and the folder it reports (OSC 7), and the daemon used to drop both. Now the daemon saves them next to the tab, in a table of their own, so the user's name for the tab is never overwritten. The tab record carries them as `program_title` and `pwd`. The app shows the program's title only as a second line on the tab, never in place of the name. A shell started again for that tab (a restart) begins in the saved folder when it is still a folder on that machine, and otherwise in the session folder as before. The manager recommends this, and it goes to the owner's next wave.
- **Reverse:** drop the `tab_program` table and the two proto fields, and the daemon goes back to ignoring the steps.

## D481: A checkpoint over the limit trims the oldest history instead of failing (TERM manager pick, 2026-11-16). Decided (owner via Fable, 2026-11-16)
A tab's screen used to fail with an internal error when its checkpoint came out over 16 MiB. Now the engine drops the oldest history rows until the checkpoint fits, keeps the newest, and the daemon says how many rows went. The holder still keeps the whole output, so nothing is lost for good. Only a screen that does not fit even with no history is an error, and it says so. The manager recommends this, and it goes to the owner's next wave.
- **Reverse:** return the error again and let the app ask for a smaller history.

## D540: Only the user reaches a machine's received place (NOPATH-2 manager pick, 2026-11-16; PL-4, D462). Decided (owner via Fable, 2026-11-16)
Files from another machine land in `<root>/received/<source machine>/`. That place belongs to no channel, so the policy treats it like Home: an agent's call that names it is refused as outside Humata's folders, and no grant covers it. The user copies or moves files from there into a channel.
- **Reverse:** let agents read the received place of their own channel's machines.

## D541: A session's `shared/<session>` folder is named after the session's folder, not its title (NOPATH-2 manager pick, 2026-11-16; PL-2, DK-11). Decided (owner via Fable, 2026-11-16)
A session's folder keeps its name when the session is renamed (SPEC, Desk), so sends follow it there and a rename never starts a second folder. The title slug is used only when the session's folder is not known.
- **Reverse:** name it by the title slug, so a renamed session starts a new folder.

## D542: Keep both numbers the new copy `name-2.ext`, then `-3`, everywhere (NOPATH-2 manager pick, 2026-11-16; FC-14, D469). Decided (owner via Fable, 2026-11-16)
The daemon's engine and the fake give the same name: the number goes before the last extension, and a name without one gets `name-2`. The old copy keeps its name.
- **Reverse:** `name 2.ext` as Finder does.

## D543: This Mac's root is stored on This Mac and reads as `this-mac` (NOPATH-2 manager pick, 2026-11-16; MACH, NOPATH). Decided (owner via Fable, 2026-11-16)
Until the user changes it, This Mac's root is the folder humatad2 started with. A change is saved in the host's store, so This Mac never shows up as a box in the machine list. `host root this-mac <path>` and the machine's id both name it.
- **Reverse:** register This Mac in the host registry and hide it from the machine list.

## D544: Changing a machine's root can move its channels, and only when nothing runs there (NOPATH-2 manager pick, 2026-11-16; MACH). Decided (owner via Fable, 2026-11-16)
The plan lists every channel and running session that the change affects. Without a move, new channels land under the new root at once and existing ones stay put. With a move, every channel folder, with its sessions, clones and conversations, goes to the new root the way a channel rename moves it. A running session refuses the move, and the same plan can still change the root for new channels only. A failure part way puts back what had moved.
- **Reverse:** move channels lazily, as their sessions next start.

## D545: Reveal on This Mac, Open terminal here on a remote, and Copy path everywhere (NOPATH-2 manager pick, 2026-11-16; PL-10). Decided (owner via Fable, 2026-11-16)
The file manager's rows and Info's clone rows offer the folder the way the machine allows. Finder can only show This Mac's files, so a remote row opens a terminal tab on that machine in that folder instead. Copy path copies the absolute path on that file's machine.
- **Reverse:** hide Reveal on remotes and offer nothing in its place.

## D546: The Inbox copy card's Somewhere else… opens the destination machine's files, and the request keeps waiting (NOPATH-2 manager pick, 2026-11-16; IN-10, D467). Decided (owner via Fable, 2026-11-16)
The card asks whether these bytes go to that machine. Somewhere else… doesn't answer: it opens the Files page on the destination machine so the user can copy the files where they want, and then deny or approve the agent's ask. The card's place reads in words ("api clone", "shared"), and the real paths and counts sit behind Details. An approve that names another place is left for later.
- **Reverse:** offer Approve and Deny only.

## D482: `:` stays a word boundary; a double-click on `path:42` copies the path (TERM manager pick, 2026-11-16; micro/12 step 3, E2E-BUGS). Decided (owner via Fable, 2026-11-16)
A double-click selects a word, and `:` ends it, so `src/parse.rs:42` copies `src/parse.rs`. That path pastes into `vim`, `less` and an editor's open box as is, while `:42` breaks most of them. The same rule keeps `host:path`, `key:value` and `https:` splits predictable. A drag still copies the line number for anyone who wants it. Recommendation: keep it.
- **Reverse:** drop `:` from `BOUNDARIES` in `terminal/view/src/selection.rs` when digits follow it.

## D483: A killed program's mouse mode stays on for the shell (TERM manager pick, 2026-11-16; micro/14 branch, E2E-BUGS). Decided (owner via Fable, 2026-11-16)
When `htop` or `vim` is killed with mouse tracking on, the shell gets the mouse reports as text until `reset` or the next program, as in most terminals. The pane cannot tell that the program that asked has gone while the shell lives on, and clearing modes on a guess would break a program that set them on purpose. ⇧ still selects and scrolls, and when the tab's own program ends the mouse goes back to the history. Recommendation: keep it.
- **Reverse:** have the holder reset mouse modes when the foreground process group changes back to the shell.

## D484: Tests compose through the tab's own text input (TERM manager pick, 2026-11-16; micro/13). Decided (owner via Fable, 2026-11-16)
A headless window has no input method, and GPUI keeps the platform's input handler private, so a live test cannot type `にほんご` the way the Japanese input method does. `humata2_ui::Shell::terminal_input` hands out the shown tab's text input, the same handler the platform calls, and the live test marks and commits text through it (`desktop/tests/live_flows/ime.rs`). It is one public method on `Shell`, read only by tests, and adds no state. A real input method's candidate window and its key ordering stay untested. Recommendation: keep it.
- **Reverse:** drop `Shell::terminal_input` and `Terminals::input_handler`, and leave composition to the pane's fakes (`terminal/pane/tests/pane.rs`).

## D485: TERM is desktop only; its browser half is cut (owner, 2026-11-16; TERM, ANYHOST, LINUX). Decided (owner via Fable, 2026-11-16)
The owner cut the browser client, ANYHOST and LINUX ("i want this app to be usable"; macOS desktop only). TERM's exit, scope and title now name the desktop app only, and its LINUX bash rows are deferred. The terminal link mapper crate stays wasm-clean (no tokio), so nothing built for the desktop has to be redone. Recommendation: keep.
- **Reverse:** reopen ANYHOST. The browser serves the same `clients/terminal_link` mapper over gRPC-web into `terminal/web`'s replica. Add the browser exit bullets back to TERM, and run the LINUX list on the Linux VM batch.

## D500: Text a person reads names a machine, never its id (STOP+UPGRADE manager pick, 2026-11-16; batch 2, UP-1, D260). Decided (owner via Fable, 2026-11-16)
Migrate item titles, reasons and card lines, the upgrade gate's refusals ("Migrate the daemon on X first" and the CLI ones) and the stop gate's ("Start sessions on X first") say "This Mac" for the host's own machine and the machine's name otherwise (today its entry name, D61), falling back to the id when no name is known. Ids stay in the structured fields: `MigrationNeeded.machine`, the card's `machine`, request machine ids. A machine-role daemon doesn't know it is "This Mac" from the host's side, so its own refusals show its id. Items restored from the Inbox keep their old text until their next update.
- **Reverse:** drop `Names` and `Migrations::set_names`, and pass `machine.value` to the titles and refusals again.

## D510: Each repository's base is picked from a chip on its own line, and the header says "Comparing per repo" only when the bases differ (VIEWS+PR manager pick, 2026-11-16; RV-31, frames 84–86). Decided (owner via Fable, 2026-11-16)
Under each repository's name sits an "against X" chip that opens that repository's menu. The menu offers the presets, then the branches the clone knew at its last fetch (under a filter), then the branch or commit typed into the filter. Only one menu, this one or the header's, is open at a time. The header reads "Comparing per repo" only when the repositories' bases really differ; overrides that all land on the same base don't count. Under another session's branch (OtherSession) the chip shows the base but is inert, since only the header's menu changes that.
- **Reverse:** per-repository bases only from the header's menu, or the header reads "Comparing per repo" whenever any override is set.
- **Amended by D739 (Fable, 2026-11-17):** when the bases agree, the chip shows only its chevron; the words appear in the header.

## D511: A push asks GitHub for the repository's own branch base, and an open pull request is not retargeted (VIEWS+PR manager pick, 2026-11-16; RV-31, PR-10). Decided (owner via Fable, 2026-11-16)
When a repository is compared with a branch, the push sends that branch, without an `origin/` prefix, as the pull request's base. A pull request that is already open keeps its base, and the push carries a note saying it wasn't retargeted and how to change it on GitHub. That note shows as an icon whose accessible name is the full note. Retargeting quietly could move a reviewed pull request under its reviewers.
- **Reverse:** retarget the open pull request to the picked branch, or always push against the default branch.

## D512: A review plan's Diff list starts with one line per repository, holding its base and push (VIEWS+PR manager pick, 2026-11-16; RV-31, RV-32). Decided (owner via Fable, 2026-11-16)
A plan group can mix repositories, so putting base chips on group headers would draw the same repository more than once. Each repository instead gets one line above the groups: its name, its "against X" chip with the same menu, and its push. No golden covers plan mode.
- **Reverse:** chips on each group header, for the repositories whose files the group holds.

## D513: Archiving a review with unsubmitted pull request drafts asks first, and offers only Archive and Cancel (VIEWS+PR manager pick, 2026-11-16; PR-17). Decided (owner via Fable, 2026-11-16)
The archive plan counts the session's pull request drafts that were never submitted (`ArchivePlan.unsubmitted_drafts`, once hard-coded to 0). Archive goes straight ahead only when nothing is unpushed and there are no such drafts. Otherwise the confirmation names them in one line ("N drafts on the pull request were never submitted; archiving keeps them"). With drafts and nothing unpushed, the dialog has Cancel and Archive: there is no work to delete or push, so the unpushed box and its two buttons don't show. No golden covers it; the fixtures have no drafts.

- 2026-11-17 (OWNERFIX, owner via Fable): archiving now keeps unsubmitted pull request drafts, restore brings them back, and only delete removes them. The dialog line says they are kept; archive-then-restore tests in the fake and the daemon pin it.
- **Reverse:** archive a review without asking, as before, and leave its drafts in the store.
- **Ruling (owner via Fable, 2026-11-16):** Change. The confirmation stays, with Cancel and Archive only, but archiving no longer drops the drafts. They stay in the store with the archived review, come back when it is restored, and go only when the review is deleted. The line says N drafts were never submitted and are kept with the archived review. Why: an archive must not lose anything the person wrote.

## D501: A box's failed holder restart shows Failed on the host (STOP+UPGRADE manager pick, 2026-11-16; batch 3, amends D378). Decided (owner via Fable, 2026-11-16)
A machine-role daemon whose restart of an agent onto the current holder fails sends the failure up in `HoldersReport.failed` (peer.proto 2, a `humata.v1.Error`): its message names the session and why, its fix says to click Migrate again or close that session to free its old holder. The host shows that machine's Holders item Failed with it as `last_error`, clears its migrating flag and keeps Migrate on offer. The machine drops the failure when it begins the next Migrate round; the next report without one returns the item to Waiting, and a report listing no old holders ends it as before (Approved if migrating, else withdrawn). The failure is still logged on the machine. Replaces D378's "the host doesn't show it Failed".
- **Reverse:** drop `HoldersReport.failed` and the machine's stored failure; the host item stays Migrating and the machine only logs.

## D502: The stop and start confirms centre on the window, like every other dialog (STOP+UPGRADE manager pick, 2026-11-16; batch 4, frames 72 and 75, reverses D363). Decided (owner via Fable, 2026-11-16)
An Opus high review of frames 72–76 against the native captures found the confirms mounted inside the machine page: centred on the page area, the sidebar left undimmed, and 94% of their width. Frames 71 and the fork dialog centre on the window. The confirms are now window-root overlays (stop at top 120, start at 180, shared scrim, Escape or a scrim click cancels), opened from the machine page or the Hosts row, and keep their set widths (520, 440). In the same batch: the Hosts card puts the stop line before the disk warning and keeps "up N" with a daemon detail, and a delivered stop on the fake ends the machine's sessions' pending requests as the daemon does.
- **Reverse:** mount the confirms in the machine page again (D363).

## D503: A stop's time reads in this Mac's local time, with the day when it wasn't today (STOP manager pick, 2026-11-16; amends D360). Decided (owner via Fable, 2026-11-16)
D360's clock read the stop's Unix time as UTC, so anywhere outside UTC the banner and the Hosts row gave the wrong hour, and a stop from an earlier day read as if it happened today. `clients/live/src/local_time.rs` turns the time into text from a `LocalNow`: the clock now plus the UTC offset from `date +%z`, kept for 10 minutes as the file manager keeps its offset. The text is "14:02" for today's local date (and for a time after now), "Tue 14:02" within the 6 days before, "14 Nov 14:02" earlier in the year, and "14 Nov 2023 14:02" before that. `map::machine` and `map::machine_stop` take the `LocalNow`; each listing, event or stop reply reads it once. The text is fixed when the machine is mapped, so a stop shown as today can read as today after midnight until the next listing or machine event. The fakes and frames keep "14:02", so nothing is re-recorded.
- **Reverse:** drop `local_time.rs` and format in UTC again as D360 did, or switch to a relative time.

## D504: The box's own stop and start points are cut by crash and by power, not crash alone (STOP manager pick, 2026-11-16; reverses D364, exit criterion 5). Decided (owner via Fable, 2026-11-16)
`stop_box_test` ends the host at each of its stop points (`stop.remote.copy_written`, `stop.remote.sent`, `stop.remote.delivered`) and start points (`start.remote.sent`, `start.remote.copy_removed`), and ends `humatad2 stop` on a box whose daemon doesn't answer at each of its points (`stop.remote.cli.record_written`, `daemon_killed`, `holders_signalled`). Each point is cut once as a crash and once as a power cut (humata2_durable's simulation on the faults build; on the box, its holders, what they ran and its agent are killed too). After the restart, the box is stopped once by that stop, the host shows it delivered, and a Start brings the session back on its conversation with a report that names the stop. A start cut shows the box running on both sides. A cut `humatad2 stop` can leave the sweep counting fewer agents than it ended, because the count is written last; the tests check only that nothing is left. The suite runs as 4 shards (`RIG_SHARDS` in `daemon/humatad/BUILD.bazel`).
- **Reverse:** drop section 5 of `tests/stop_box.rs` and `RIG_SHARDS`; D364 stands again.

## D520: The handover summary reads a Codex rollout by its lines' shape, not by the kind's format (E2E-BUGS manager pick, 2026-11-16; FK-6, agents-10 step 12). Decided (owner via Fable, 2026-11-16)
`digest_of` in `daemon/humatad/src/session_service/start_fresh.rs` reads both Claude JSONL lines and a Codex rollout's `event_msg` `item_completed` lines (`UserMessage` as the user, `AgentMessage` as the agent). Codex's injected `AGENTS.md` and environment text are `response_item` lines and are left out, as Claude's meta lines are. The summary is a best-effort digest, so reading by shape needs no format to be passed down, and a kind that names no `conversation_format` still gets one. A fork before a given turn of a Codex parent still falls back to the terminal's output, because turn offsets come from `humata2_mirror::turns_of`, which reads Claude's shape; that waits until a Codex kind is built in.
- **Reverse:** pass the parent kind's `conversation_format` to `handover_digest` and read through `humata2_conversation`'s adapter for that format.

## D560: A channel's Add a GitHub repo… is one sheet built on the one repo search (REPOS manager pick, 2026-11-16; CH-6, CH-18, RL-5, D460). Decided (owner via Fable, 2026-11-16)
The Repos and machines tab's Add a GitHub repo… buttons open a sheet that embeds the shared repo search (`RepoPicker::ChannelAddRepo`) with plain click rows. Picking a repo adds it and closes the sheet. An error shows inline and the sheet stays open. A repo the channel already has is refused inline, so a pick never starts a re-clone. The sheet shows no path and has no local-folder option.
- **Reverse:** drop `Overlay::ChannelAddRepo` and disable the two stubs again.

## D561: The client adds a repo to a channel by owner/name only; a pasted address is read on the client (REPOS manager pick, 2026-11-16; CH-18, KP-4, D460). Decided (owner via Fable, 2026-11-16)
`Daemon::add_repo_to_channel(channel, "owner/name")` calls ChannelService.AddRepo and waits for its operation. The live call doesn't publish a ChannelSetup event, so a New channel sheet can't take it for its own setup. The pure `github::parse_repo_text` reads a pasted GitHub address (https, ssh, `git@github.com:`, a trailing `.git` or branch path) as owner/name. It refuses paths, `~`, `..` and other hosts. The palette offers "Add owner/name to this channel…" for such text while a channel is on screen.
- **Reverse:** accept only typed owner/name and drop the palette entry.

## D562: Adding a repo the channel lacks to a session adds it to the channel first (REPOS manager pick, 2026-11-16; CH-21, CH-18, CH-19). Decided (owner via Fable, 2026-11-16)
A user's AddRepoToSession for a repo the channel doesn't have goes through the channel's own add (`ChannelSvc::add_repo_to_channel`). That add clones on every machine of the channel and records the repo in the library. The session add waits for this machine's main clone, then makes the session's clone from it on the session's branch (FC-11). A failed main clone is retried through the same add. If it fails again, the operation fails at its `add_to_channel` step, and the channel keeps the repo with a failed main clone, as a channel add does today. A session on another machine isn't served yet.
- **Reverse:** refuse with "add it to the channel first" and drop the `add_to_channel` step.

## D563: An agent adds repos only to its own session, only from its channel, and is asked first (REPOS manager pick, 2026-11-16; CH-21, CH-26, D479). Decided (owner via Fable, 2026-11-16)
An agent may add a repo only to its own session, under OWN_SESSION_MANAGE (Ask by default), and only one its channel already has. For a repo the channel lacks, the agent is refused and pointed to `channel_add_repo` with its also-to-session flag, so the channel's proposal stays the only door for agents (CH-26). The operation reuses OperationKind ADD_REPO with the session as its target, so the proto doesn't change.
- **Reverse:** let an agent's session add carry the channel add, behind the same ask.

## D564: New desk session uses the one repo search, held in AppState (REPOS manager pick, 2026-11-16; DK-2, RL-5). Decided (owner via Fable, 2026-11-16)
`features/desk` can't depend on `features/app`'s RepoSearch, because app depends on desk. The New desk session sheet's search therefore lives in `AppState.new_desk_repos` and is routed by `RepoPicker::NewDeskSession`, like the Desk's Add repo search (`desk_repos`). The desk form gains only `NewAction::ToggleRepo`: a tick adds or removes a repo, and the field stays open until Done. Return on typed text still adds it. The Folder row and its caption are unchanged (DOCV).
- **Reverse:** move RepoSearch into a crate both features use.

## D565: Org suggestions come from the library and from gh's list of the user's orgs (REPOS manager pick, 2026-11-16; RL-8, RL-9). Decided (owner via Fable, 2026-11-16)
RepoService.ListRepoOwners is a global read that runs `gh api user/orgs` on the host. A gh failure comes back as the answer's problem, never a failed call, and the suggestions then fall back to the library's owners. One rule, `clients::repos::suggest_owners`, serves the search field's + Org and Settings › Repositories: library owners first (most recently used first), then GitHub orgs, de-duplicated ignoring case, without owners already in the scope. Each picker asks once per opening.
- **Reverse:** suggest library owners only and drop ListRepoOwners.

## D566: "used … ago" is worked out from a now the reducer is given (REPOS manager pick, 2026-11-16; RL-3). Decided (owner via Fable, 2026-11-16)
The TCA clock is monotonic and has no epoch, so it can't be compared with a library row's `last_used_at_ms`. The reducers instead stamp `now_ms` into state from `Daemon::now_ms()`. The live client returns this machine's wall clock; the fake returns a settable fixed time. The UI no longer reads the clock itself, and the wording helpers are pure functions of (`last_used_at_ms`, `now_ms`).
- **Reverse:** read the wall clock in the view again.

## D567: The CLI can forget the library and save the scope (REPOS manager pick, 2026-11-16; RL-4, RL-7, D479; reverses B5's "the app's, not the CLI's"). Decided (owner via Fable, 2026-11-16)
The owner wants everything the app does reachable from the CLI. `humata repos forget-all` first lists the library, then says how many repos go and that nothing on disk or GitHub is deleted. It asks "Continue?" like any plan, needs `--yes` without a terminal, and with `--dry-run` only says what it would do. `humata repos scope show` prints the saved scope. `humata repos scope set [--owner o]… [--no-include-me]` saves owners plus your own repos. With no owner it saves your repos, and it refuses `--no-include-me` alone. Both changes stay user-only (D479).
- **Reverse:** drop forget-all and scope set from the CLI and leave them to Settings.

## D522: Making or revoking a script token needs the user and no second presence step (E2E-BUGS manager pick, 2026-11-16; SEC-13, SEC-14, agents-18). Decided (owner via Fable, 2026-11-16)
`CreateScriptToken` and `RevokeScriptToken` take a user caller only (`user_only` in `daemon/humatad/src/cli_login.rs`): the app, or a terminal whose `humata login` the user confirmed in the app or with the approval passphrase. That login is the presence step, so the daemon does not ask again, and the CLI's `present` path makes one call. An agent or a script is refused. For the owner's decision wave.
- **Reverse:** have both handlers answer `PresenceRequired` (confirm in the app, or the passphrase while the host is away) when a proof is missing; the CLI already retries with one.

## D523: A script token's channels limit channel actions, not a machine stop (E2E-BUGS manager pick, 2026-11-16; SEC-14, agents-18 step 7). Decided (owner via Fable, 2026-11-16)
A token with `machine_sessions_stop` stops this machine's sessions whatever channels it lists (`check_script` in `daemon/access/src/policy.rs` skips the channel check for a machine target). A machine stop is not a channel action, and a cron stop that had to list every channel would break each time one is added. Proven by `daemon/humatad/tests/stop_access.rs::a_script_whose_token_lists_stopping_sessions_stops_the_machine_and_is_named_a_script`. For the owner's decision wave.
- **Reverse:** refuse a machine stop unless the token lists every channel with a session on the machine.

## D524: OpenCode's turn end comes from its server's events; SPEC SE-21 lags (E2E-BUGS manager pick, 2026-11-16; D201, SE-21, SE-23, agents-17). Decided (owner via Fable, 2026-11-16)
Since D201 OpenCode reports its turns through the hooks path, the built-in agents table (`daemon/sessions/src/agents.rs`) marks it hook-backed, and the app sends held messages at its reported turn end (`flush` in `features/session/src/lib.rs`). SPEC SE-21 and its agents table listed OpenCode on the output fallback; they now put it beside Claude Code and Copilot CLI (E2E-BUGS e2e_4). A session that is only Probably idle never sends on its own, whatever its agent.
- **Reverse:** drop OpenCode from `flush`'s agents, so its held messages wait for Send now as SE-21 reads today.

## D550: The move step card shows a thin bar and "n of m MB" under the running copy step (ROAM manager pick, 2026-11-16; MV-17, draft frame 97). Decided (owner via Fable, 2026-11-16)
A long copy looked frozen with only a step title. The running copy step now carries a 180 by 3 bar in the line and accent colours, with "62 of 118 MB" beside it in the secondary colour, drawn from `MoveProgress.done_units` and `total_units`. Steps without units show no bar. The owner decides whether the bar stays.
- **Reverse:** drop `StepRow.progress` and `copy_progress`; the step card shows titles only again.

## D551: Continue in another channel reaches the CLI and MCP (ROAM manager pick, 2026-11-16; CV-8)
The CLI is `humata session continue <session> --channel <c> [--before-turn n]` (op id `session.continue_in_channel`), and MCP has `session_continue_in_channel`, which asks first like `session_fork`. The global `--channel` names the target. Leaving it out is a usage error, never the agent's own channel, because the daemon would only refuse that.
- **Reverse:** remove the subcommand and the MCP tool; the RPC stays.

## D552: A channel rename gives every known folder its exact new Claude project key, long keys included (ROAM manager pick, 2026-11-16; CH rename, D336)
The session folder, clones and launch folder the records know about get their exact new key, through `claude_project_name`, in the store records, the conversation moves and the rewritten file contents. Claude's key is a 200-unit cut plus a hash of the whole path, so no prefix replace can rebuild a long one. A folder no record names keeps the prefix rule. Folders nested deeper than sessions/<name>/<clone> are not yet covered.
- **Reverse:** go back to the prefix replace in `Placement`; long keys then go stale on rename.

## D553: "Continue in another channel…" sits in the session menu under Move, hidden on the Desk (ROAM manager pick, 2026-11-16; CV-8, D307). Decided (owner via Fable, 2026-11-16)
Its dialog picks a channel the way Move picks a machine, with the session's own channel dimmed as "here now". It picks a "Continue from" point the way Fork does: Now, or before turn n for n of 2 or more. Continue opens the new session. No frame draws this dialog, so it is built from existing components with no new chrome, and draft frame 113-continue-in-channel records it for the owner.
- **Reverse:** drop the menu item and `overlay_continue.rs`; continue stays on the CLI and MCP.

## D554: A session on another machine continues there, journaled on the host before that machine is asked (ROAM manager pick, 2026-11-16; CV-8, D432). Decided (owner via Fable, 2026-11-16)
It continues on the machine that holds its conversation and clones, into a channel that machine is in; a channel it isn't in is refused before anything changes, since carrying the session across machines is a separate move. The host keeps the ask in `<state>/carry/remote_archive/continue/<op>.json` until its records are written, or until the machine made nothing or was forgotten. A link lost part way, or a host restart while the link is down, is finished when the link comes back: the host asks again under the same key and writes the records once. The record's start names the host operation, so replaying the caller's key while the continue runs answers with the session the machine named. Why: a continue the host forgot would leave a session running on the machine unlisted, and keeping the ask before making it means a cut at any point is made whole or not at all. Provisional: the restart report kind is `remote_continue`, and a journal that can't be read is logged and left for the owner, not retried.
- **Reverse:** refuse sessions on another machine again (`not_built`).

## D555: A continue writes its copy note before its records, and its session and start in one write (ROAM manager pick, 2026-11-16; CV-8)
`record_and_carry` writes the `.humata/fork-copy-*` note first, then the session and its start in one store write (`put_session_with_detail`). At restart, a session folder that no record names and that holds only copy notes is taken back, along with any copy the notes name. No crash point can leave a session that a restart would start as a plain new session.
- **Reverse:** write the records first again; the crash window between the note and `put_session` comes back.

## D556: Turns, fork-before-turn and continue go through each CLI's conversation adapter, for Claude, Codex and Copilot (ROAM manager pick, 2026-11-16; CV-3, CV-8, lifts D520). Decided (owner via Fable, 2026-11-16)
Listing turns, forking before a turn and continuing in a channel all read the conversation through its CLI's adapter. A format the adapter calls Unsupported is refused as AgentLacksFeature before anything changes, for a context fork from now or before a turn, locally or staged from the host. A handover before a turn cuts both the handed file and its summary where the parent's format says that turn starts (the adapter's turn starts for Codex and Copilot, the mirror's offsets for Claude), and a fork carried to another machine is cut the same way. Each fresh launch (start, fork, rebuild) runs the kind's version_args once (by default its adapter's, such as `--version` for Codex; an agents file may set its own) and keeps the first line as the session's `cli_version`; resumes keep it, a fork carried to another machine keeps its parent's, and the adapter's check gets the recorded version with no guessed fallback. Provisional: the mirror fallback for Claude conversations kept elsewhere, ContextFork as the fork's feature, and a remote raw copy still carrying the parent's id inside the conversation.
- **Reverse:** restore `reads_turns` and the Copilot refusal; Codex and Copilot handovers before a turn then hand the whole conversation again.

## D557: The palette offers "Continue in another channel…" where the session menu does (ROAM manager pick, 2026-11-16; CV-8, D553)
It is listed only over a live channel session: hidden on the Desk and for archived sessions, like the menu item. It opens the same dialog. CV-8's draft frame is numbered 113-continue-in-channel, the lowest free number from 113 up, so it no longer shares 98.
- **Reverse:** drop `Command::ContinueInChannel`.

## D558: A channel has its own Desk tab and sidebar row, and closing a channel Desk session always asks first (ROAM manager pick, 2026-11-16; CD-2 to CD-6, D306, frames 91-95). Decided (owner via Fable, 2026-11-16)
A channel's Desk sessions load per channel and never enter the plain Desk or the Sessions tree. The channel page shows "Desk · n" after Sessions, with an empty state offering New desk session, and the sidebar shows a Desk row under the channel while it has open channel Desk sessions. Closing from the session's own page returns to the channel's Desk tab. A switch whose channel rule is Never shows "Blocked by the channel rule: Never" whether on or off; the Ask/Never rows themselves are unchanged. Each row leaves a slot for DOCV's doc history strip.
- **Reverse:** drop the channel Desk tab and sidebar row; channel Desk sessions stay reachable from the CLI and MCP only.

## D559: The channel Desk matches frames 91 to 95, and adds what CD-2, CD-5 and CD-6 ask that no frame draws (ROAM manager pick, 2026-11-16; D558). Decided (owner via Fable, 2026-11-16)
Its switches are pill toggles, the session header reads "# channel · Desk", and a blocked switch's "Agent access" link opens the channel's Agent access tab from the New dialog and the Info panel. The Close confirmation shows a line count per Markdown file, sent by the daemon as DeskSession `document_lines` (field 15) from a bounded read of at most 8 MiB per file, only when a channel Desk session is listed; a file it can't read or that is too large shows no count, and a Desk on another machine passes through that machine's counts. Without frames, as drafts: a session's Info panel lists the channel Desk sessions that target it (CD-2), "New desk session for this session" sits in a session's menu (CD-6), a session whose target went shows "Target gone" with Close (CD-5), and the sidebar cursor walks the channel Desk rows. Provisional because of the drafts.
- **Reverse:** drop the drafts; the frames' parts stay.

## D670: Codex is built in, and its conversation is found in its session folder once its CLI writes it (AGENTKINDS manager pick, 2026-11-16; B9). Decided (owner via Fable, 2026-11-16)
The built-in agents are Claude, Codex, Copilot and OpenCode, in that order. Copilot 1.0.113 creates exactly the id it is given with `--session-id` and resumes it with the same flag, so it is pinned like Claude. Codex 0.169.0 has no flag to set an id, so a kind can say `conversation_id = "found-in-folder"`. A launch whose conversation file isn't there yet learns its conversation from the CLI's own folder: the newest rollout whose `session_meta` cwd is the session folder and whose id no other session's agent holds. The launch then names that id in its resume command and file. It learns on each turn report, on a resume the person asks for, before a fork and before a restart brings agents back. An agents file that says nothing stays pinned. Why: Humata can't choose the id, and the folder is the only thing it knows that the CLI records. Provisional: two Codex runs in one folder at once (for example a fork sharing it) are told apart only by the ids already taken.
- **Reverse:** drop `found-in-folder`; Codex sessions then resume only with `codex resume --last` semantics, or not at all.

## D671: A resume the person asks for isn't told it was interrupted, and typed text is submitted as its own keystroke (AGENTKINDS manager pick, 2026-11-16; B7, B12). Decided (owner via Fable, 2026-11-16)
An agent that was Stopped or had exited and is resumed on the person's ask records no Interruption and gets no note. Verify checks only that it came up, stayed up and started no other conversation. Revival after a crash or a lost holder still tells. Text sent to an agent that ends in a newline is typed without it; 400 ms later a return is sent as its own write. The same pause applies to the first message and to notes. Under an idempotency key the return is a second step, `<key>/submit`, so a retry sends only the steps that never arrived. Why: the agents' TUIs take a pasted return as part of the paste, and a resume the person asked for isn't a failure. Provisional: the 400 ms pause.
- **Reverse:** type the return with the text again, and tell every resume.

## D672: `session start` defaults to this machine, `host ls` lists it, and attach waits for a resuming tab (AGENTKINDS manager pick, 2026-11-16; B3, B10). Decided (owner via Fable, 2026-11-16)
`humata host ls` puts this machine first as "This Mac", using its id from GetServerInfo, since ListMachines holds only other machines. `humata session start` without `--machine` starts there, and `--machine this-mac` names it. Review, files and boards still need `--machine`. An attach that finds a holder its tab doesn't name yet, as happens while a resume is saving it, looks again for up to 5 s before refusing. Why: the owner's flows start and attach on this machine first. Provisional: the polling wait, which a notify could replace.
- **Reverse:** require `--machine` again, and refuse the attach at once.
- **Ruling (owner via Fable, 2026-11-16):** Change. `session start` defaulting to this machine and `host ls` listing it stand. The attach's look-again loop goes: the attach waits on the tab's change notification for the holder to be named, bounded at 5 s, and then refuses as it does now. Why: nothing polls where a notification exists.

## D673: Copilot's hook and permission records are known from the CLI's own event schema (AGENTKINDS manager pick, 2026-11-16; B8). Decided (owner via Fable, 2026-11-16)
The Copilot adapter's strict parse now knows `hook.start`, `hook.end`, `permission.requested` and `permission.completed`. Their `data` fields come from the session-event schema that ships inside the CLI package (1.0.113-1), not from a captured run. A fork or a carry rewrites them like any other line, and a field the parse doesn't know still refuses the conversation when it holds a path. Why: the owner's real sessions hold these records, so without them every fork and carry of those sessions was refused. Provisional: marked X2 until a real-CLI capture with a hook and a permission prompt confirms the shapes.
- **Reverse:** drop the four entries; sessions holding them are refused again.

## D674: An error's chain names each cause once (AGENTKINDS manager pick, 2026-11-16; the fork error printed twice). Decided (owner via Fable, 2026-11-16)
`status::chain` adds a cause only when the text so far doesn't already say it. Many messages include `{source}` themselves, so the fork error and others had printed the reason twice. Provisional: a cause whose text happens to appear elsewhere in the message is left out.
- **Reverse:** append every cause again, and drop `{source}` from the messages instead.

## D675: The history mirror learns a conversation its agent wrote before the session named it (AGENTKINDS manager pick, 2026-11-16; B9). Decided (owner via Fable, 2026-11-16)
HistoryService holds the agents and their home. Its catch-up and the host's ReadMirror run the same learning as a resume before they read the agent's file. A Codex session therefore shows up in search and in `humata session read` after its first turn, with no resume first. Why: the mirror follows Session events and never scans, so it needs the bound file. Provisional: the learn runs on each catch-up of an unbound session, which is one directory walk of the CLI's sessions folder.
- **Reverse:** learn only on resume, fork, turn report and restart.

## D630: Re-attach shows only on an outdated draft (VIEWS+PR manager pick, 2026-11-16; PR-14, frame 68). Decided (owner via Fable, 2026-11-16)
A draft whose anchor still matches its PR's head has Edit and Drop only; one whose lines moved or went shows Re-attach, which opens the line picker on the current head.
- **Reverse:** show Re-attach on every draft.

## D631: A failed Ask shows its error under the review chat that asked, with a dismiss (VIEWS+PR manager pick, 2026-11-16; PR-12, frame 68). Decided (owner via Fable, 2026-11-16)
The error belongs to the session the Ask was sent to and clears on dismiss or on the next Ask; other reviews show nothing.
- **Reverse:** show the error as a toast.

## D632: The bottom-up order comes from the names in the diff text (VIEWS+PR manager pick, 2026-11-16; PR-4 step 4). Decided (owner via Fable, 2026-11-16)
Files and hunks are walked from callees up to callers by matching the names each hunk defines against the names other hunks use; a file with no match keeps its diff place.
- **Reverse:** keep the diff's own order.

## D633: A review on a machine is gated like a review here (VIEWS+PR manager pick, 2026-11-16; §26.17, PR-2). Decided (owner via Fable, 2026-11-16)
Starting a review whose session lives on another machine asks the same Agent access switch and makes the clone there through the session carry service.
- **Reverse:** allow remote reviews without the switch.

## D634: A review's clones sit at the machine's host-layout paths, and git is routed by session and repo (VIEWS+PR manager pick, 2026-11-16; §26.17, D633). Decided (owner via Fable, 2026-11-16)
The daemon finds which machine holds a review's clone from the store; a store error is shown, never a silent fall back to local git.
- **Reverse:** clone every review on the host.

## D635: Ask sits on hunk headers and file rows, shown on hover (VIEWS+PR manager pick, 2026-11-16; PR-12). Decided (owner via Fable, 2026-11-16)
Ask on a hunk anchors the comment box to the hunk's new-side lines; Ask on a file anchors it to the file. The buttons are hidden until the row is hovered.
- **Reverse:** show Ask only on single lines.

## D636: HeadChange.stack comes from the review's own pull requests (VIEWS+PR manager pick, 2026-11-16; PR-16). Decided (owner via Fable, 2026-11-16)
The parent and children are the review's PRs whose base and head branches chain; RepoStack is not consulted yet.
- **Reverse:** read the stack from RepoStack.

## D637: Moving a review carries its PR bindings with the session's store rows, and its review refs in their own bundle (VIEWS+PR manager pick, 2026-11-16; PR-21, MV-13). Decided (owner via Fable, 2026-11-16)
The review sessions and their PRs ride the session snapshot; refs not on any remote go in a separate git bundle.
- **Reverse:** re-fetch the PR refs on the target instead.

## D638: The move's check step makes the review channel on the target first (VIEWS+PR manager pick, 2026-11-16; PR-21). Decided (owner via Fable, 2026-11-16)
If the channel can't be made, or the target machine was removed, the move stops before anything is copied, with an error that says which.
- **Reverse:** make the channel after the copy.

## D639: A moved review whose GitHub is unreachable is rebuilt like any other carry (VIEWS+PR manager pick, 2026-11-16; PR-21). Decided (owner via Fable, 2026-11-16)
No review-only error; the existing rebuild path and its message apply.
- **Reverse:** add a review-specific error.

## D620: ListViews `include_forks` walks the store's fork origins (VIEWS manager pick, 2026-11-16; VW-10, VW-14). Decided (owner via Fable, 2026-11-16)
The daemon lists the session's views with those of every fork below it, read from `fork_origins` (each session once, so a cycle ends the walk), newest first by when each was prepared. A fork's drafts show only to that fork's own agent; a fork the caller may not read, or one the store no longer holds, is left out, not refused, as a board does. Provisional because the Journal still chains forks itself.
- **Reverse:** list the session's own views only, as before.

## D621: A review plan's notes are re-checked when views are listed, not stored again (VIEWS manager pick, 2026-11-16; RV-34). Decided (owner via Fable, 2026-11-16)
ListViews reads each review plan's notes against its own session's clones as they are now, so a note whose lines changed reads "outdated" and one whose lines only moved shows its new lines; the stored view keeps the notes as prepared, and a plan whose clones can't be read now (a box's session listed on the host, say) is listed as prepared. The app reads the plan again when it opens the view, and the Changes panel draws each note as "L12–14 · text" with an outdated tag. Provisional until the owner sees it.
- **Reverse:** show the notes as prepared, as before.

## D622: A remote push the host kept but never sent ends Failed after a host restart, and is never sent twice (VIEWS manager pick, 2026-11-16; F3)
If the host stops after keeping a push and before the box confirms it, the restart can't tell whether the box heard it, so the push ends Failed and says to look at the box and push again. The same idempotency key asked again after a restart answers the push already running, and the box opens one pull request. This is the code's behaviour; the tests pin it.
- **Reverse:** none needed; to retry instead, the host would first have to ask the box whether it has the push.

## D655: `humata cp|mv … <machine>` places in the CLI, one transfer per landing place, and keeps both by default (PLACES manager pick, 2026-11-16; PL-3, PL-12, PL-13). Decided (owner via Fable, 2026-11-16)
The CLI reads the sessions and the source's channel and works out each source's landing by the same rule as the Files page's Copy to and Move to, so the daemon's StartTransfer is unchanged. Sources that land in different places start one transfer each. The first transfer uses the command's idempotency key and each later one adds `/n` to it, so a retry repeats every one. With no `--on-existing`, a machine-only send keeps both (PL-13), while a send to a location still fails on a taken name. The last word is a machine only when it has no `:` and no `/`. Inside a session it is a machine only if one has that name; otherwise it is the agent's own clone path, and `name/` forces a clone folder. Why: the person never types a path (PL-12), and an agent's bare clone paths keep working.
- **Reverse:** move the rule into the daemon as a machine-only destination on StartTransferRequest; the CLI then sends the machine alone.

## D568: A machine forwards its agents' repo searches to the host and keeps no copy (PLACES for REPOS, 2026-11-16; RL-11, RL-12, D227, D473). Decided (owner via Fable, 2026-11-16)
A machine's daemon used to answer every RepoService call with "not built", so an agent on devbox-1 couldn't search at all. Now its `SearchRepos` and `ListRepoLibrary` go up the Uplink the way board calls do (D227). The host runs them as that agent, under its rule for global reads. While the host is away, the call is `host_unreachable` with RL-12's words, and no last copy is kept: a copy would offer repos the user has since removed. Removing a repo, forgetting the library, the scope and the org list stay on the host, and a machine refuses them with a fix that points there. The user's own `humata repos search` over SSH, outside every session, is refused the same way, because AWAY cut MachineUser forwarding.
- **Reverse:** register RepoService on machines as not built again, and take the two rows out of the forwarder's table.

## D610: An unreadable continue-in journal stops as stuck and is kept beside it as .bad (ROAM manager pick, 2026-11-16; D559, restart reports). Decided (owner via Fable, 2026-11-16)
A journal file that can't be read, isn't JSON, or has the wrong shape goes into the restart report as remote_continue, step asked, state stuck. The file is renamed to `<op>.json.bad` (then .bad2 and on) and the folder is synced, so nothing is lost and the next restart doesn't trip on it again. The report is the only place it shows; there is no automatic retry.
- **Reverse:** delete the bad journal instead of keeping it, or retry the step on the next restart.

## D611: A resume asks the CLI its version again, and a wrapped CLI records its own version (ROAM manager pick, 2026-11-16; D559, revive). Decided (owner via Fable, 2026-11-16)
resume() asks the CLI's version (CliVersions) before it saves "resuming", so a CLI upgraded since the start is seen before it runs. A wrapper's CLI is the leading args shared by start_args and resume_args, up to the first that starts with "-" or contains "{"; that command is the one asked for its version.
- **Reverse:** keep the version recorded at start, and treat a wrapper as one opaque command.

## D612: A channel rename moves every Codex and Copilot conversation to its new deep folder through the agent's adapter (ROAM manager pick, 2026-11-16; channel rename, deep folder keys). Decided (owner via Fable, 2026-11-16)
follow_conversations runs in finish_move and goes through AgentKind::follow_folder, which uses adapter.place, never a directory walk or a text rewrite. An earlier conversation moves to the folder it recorded itself (Codex session_meta payload.cwd, Copilot session.start data.context.cwd); one that recorded none follows by its ancestor folders. The Codex thread row follows too. A file that exists but isn't where the CLI keeps it stops the rename with FollowError::Misplaced and says to put it back or remove it; a missing file is skipped.
- **Reverse:** rewrite only the current conversation and leave earlier ones where they were.

## D613: DeskClient::target_sessions stays, though the Info panel now reads the workspace (ROAM manager pick, 2026-11-16; CD-2, D559). Decided (owner via Fable, 2026-11-16)
The Info panel's list of Desk sessions looking after a session is derived from the workspace, so the app's ListTargeting cache is gone. target_sessions stays on the client for the CLI and MCP, which have no workspace.
- **Reverse:** remove target_sessions and its rpc.

## D614: A copy to another machine re-ids its conversation through fork_conversation, with no raw-copy fallback (ROAM manager pick, 2026-11-16; carry, staged fork). Decided (owner via Fable, 2026-11-16)
The staged remote fork (carry_fork stage_fork) was the one path that raw-copied a conversation. It now goes through forked_for_stage, which calls fork_conversation, so the copy gets its own id on every kind with an adapter (Claude, Copilot, Codex). A kind with an adapter whose file is misplaced fails with a typed NotFound instead of copying anyway; a pack that keeps its id keeps it.
- **Reverse:** raw-copy and accept two sessions sharing one conversation id.

## D615: The fake Desk keeps a closed session's folder only when it has saved versions (ROAM manager pick, 2026-11-16; DV-13, DOCV 64). Decided (owner via Fable, 2026-11-16)
The fake's close keeps the folder when version_count > 0, matching DV-13 and DOCV 64b's version-only folder_kept. Reopen covers channel Desk sessions as it does plain ones.
- **Reverse:** keep any folder with Markdown in it.

## D616: Retargeting has its own rpc, the daemon alone decides Target gone, and a retarget resumes what it stopped (ROAM manager pick, 2026-11-16; CD-5, D559). Decided (owner via Fable, 2026-11-16)
Desk.SetDeskTarget(session, target) is separate from SetDeskAccess, since access and target differ and the CLI and MCP name the call. The daemon's watch marks target_gone when the target is deleted, archived past the undo window, or no longer in the channel; reopen and restore use the same check, and the app shows only what the daemon says. A Desk folder with no Markdown is still closed rather than flagged (CD-5). Retargeting or clearing a session stopped for Target gone resumes it: locally through resume_saved, remotely through the existing remote resume HostOp; an offline link refuses the retarget and changes nothing. The sidebar's channel Desk header is a key stop.
- **Reverse:** put retarget back on SetDeskAccess, or derive gone in the app.

## D617: The Desk follows daemon events instead of re-reading (ROAM manager pick, 2026-11-16; flow 1, D559). Decided (owner via Fable, 2026-11-16)
Event.desk = 33 carries a DeskSession (upsert) or a removal with only the session set, reusing the DeskSession message and the Session payload scope. The daemon follows its own Session events in desk_service/follow.rs, because every Desk write already tells one, so a single hook covers start, close, reopen, keep, access, repos, versions, moves, retarget and channel close/restore; clean-up publishes its own Gone. Sessions on other machines are filled from that machine as the list does, so they push too. The app applies events to workspace.desk in place; its post-action load() stays, since it also brings session rows.
- **Reverse:** drop the event and re-read the Desk list after each change.

## D618: The fake daemon tells Desk changes from one place, and its move moves the scratch folder (ROAM manager pick, 2026-11-16; D617). Decided (owner via Fable, 2026-11-16)
Inner::emit diffs DeskData against what it last told and sends Desk events before the event itself, so every fake path, including remove_session and relocate_session, tells the app. The fake's move puts the scratch folder under the new machine's root, as the daemon does, so a move is a Desk change.
- **Reverse:** emit Desk events per Desk call only.

## D619: Copilot forks and carries are proven on the real CLI for tools, subagents, compaction, rewind and detached heads; the rest stays refused (ROAM manager pick, 2026-11-16; X1, X2). Decided (owner via Fable, 2026-11-16)
Proven against Copilot CLI 1.0.113-1 in a temp HOME (captures under daemon/conversation/testdata/copilot/1.0.113-1): a tool call splits a turn, so a turn finishes only at an assistant.turn_end after a reply with no tool request; a subagent's prompt (with source) is not a turn; a compaction's summary survives a fork cut after it; /rewind rewrites events.jsonl in place; multi-line names are YAML block scalars. session.db is carried byte for byte only when it is SQLite and names none of the old folder, Copilot dir or id; any other binary file is still refused. A detached head is written as detached@<sha7> with headCommit set, and any branch YAML could misread is single-quoted, even where the CLI leaves it plain, since it reads back the same. Hooks, skills, MCP, plan mode, permission prompts, background tasks, workflows and a session.db with rows stay refused until proven. The X1 minimum stays 1.0.112.
- **Reverse:** refuse tool, subagent and compacted conversations again, as X1 did.

## D680: Machine scripts live in ~/.humata2/config/machine-scripts.toml and ~/.humata2/scripts, named, never pathed (MACHINESCRIPTS manager pick, 2026-11-16; TS-3, D411, portable config, PL). Decided (owner via Fable, 2026-11-16)
One rules file on the host, synced to machines like stack.toml (secret filter applied); scripts in a Humata-owned folder named by file name only, streamed to `bash -s` over the link on each run, never stored on machines. No host variables reach scripts; they read the machine's login environment.
- **Reverse:** store scripts on each machine, or let rules name paths.

## D681: Four triggers: added, connected, session-arrived, session-leaving (MACHINESCRIPTS manager pick, 2026-11-16). Decided (owner via Fable, 2026-11-16)
`added` replaces `connected` on the first link-up after Add machine; `connected` fires once per later link-up and queues behind a run still going; `session-arrived` after a move ends (recovered moves too) on the new machine; `session-leaving` after the copy is verified and before the old agent stops, on the old machine. This Mac has none.
- **Reverse:** fire `connected` on the first link-up too, or let session-leaving hold the move.

## D682: Steps are run (script), humata (CLI argv), start (agent kind in a named channel) and send (type once idle) (MACHINESCRIPTS manager pick, 2026-11-16; SE-21, AGENTKINDS). Decided (owner via Fable, 2026-11-16)
`send` waits for SE-21 idle (default 2 min) and is keyed so a retry never types twice; a kind with no turn end fails rather than typing blind unless `wait = "none"`. `start` uses StartSession and is refused on session triggers. Kinds come from Settings and built-ins; none are hard-coded.
- **Reverse:** allow typing into every session of a kind on a machine trigger.

## D683: Rules run in file order, steps in order, the first failed step stops its rule; `machines` defaults to all (MACHINESCRIPTS manager pick, 2026-11-16). Decided (owner via Fable, 2026-11-16)
An unknown machine name in `machines` is a check warning, since it may be added later.
- **Reverse:** run rules in parallel, or continue after a failed step.

## D684: Timeouts 5 min a step, rule cap up to 1 h, 60 s a session-leaving step; `once = true` runs a rule once per machine (MACHINESCRIPTS manager pick, 2026-11-16). Decided (owner via Fable, 2026-11-16)
A timed-out step is killed and fails. `once` records the first pass on the host; a failure retries at the next trigger. Without `once`, scripts should be idempotent.
- **Reverse:** no `once`; longer default timeouts.

## D685: Scripts get HUMATA_TRIGGER, HUMATA_MACHINE and, for session triggers, HUMATA_SESSION, HUMATA_CHANNEL, HUMATA_SESSION_DIR, HUMATA_FROM/HUMATA_TO (MACHINESCRIPTS manager pick, 2026-11-16). Decided (owner via Fable, 2026-11-16)
- **Reverse:** pass these as arguments instead.

## D686: Results: a host run log (last 50 a machine, 40-line output tails), a Scripts section on the machine page, and one Inbox host ask per (machine, rule) only on failure, with Retry (MACHINESCRIPTS manager pick, 2026-11-16; D348). Decided (owner via Fable, 2026-11-16)
A newer failure replaces the item; a later pass withdraws it. Nothing polls: the feed carries `MachineScriptRun`.
- **Reverse:** show passes in the Inbox too, or post a card per failure.

## D687: CLI `humata machine script list|check|run|log`; MachineService gets ListMachineScripts, CheckMachineScripts, RunMachineScript (Operation) and ListMachineScriptRuns; nothing writes rules (MACHINESCRIPTS manager pick, 2026-11-16). Decided (owner via Fable, 2026-11-16)
- **Reverse:** a separate MachineScriptService, or rule editing over gRPC.

## D688: Triggered runs are the owner's automation and need no access check; an agent's Run now needs `machine_script_run`, NeverByDefault; peers can't call any of it (MACHINESCRIPTS manager pick, 2026-11-16; access switches). Decided (owner via Fable, 2026-11-16)
Reads (list, check, log) follow `machine_read`. No RPC, CLI or MCP tool writes rules or scripts.
- **Reverse:** Ask by default for Run now.

## D689: Open: a "machine upgraded" trigger, and starting agents from TS-3 sync rules (MACHINESCRIPTS manager, 2026-11-16). Decided (owner via Fable, 2026-11-16)
Left out until UPGRADE's Migrate ends with a hook the run engine can hear. Recommendation: add `on = "upgraded"` firing after a machine's Migrate succeeds.
- **Reverse:** n/a (nothing built).
- **Ruling (owner via Fable, 2026-11-16):** Answered. Build `on = "upgraded"`: it fires once on a machine after its Migrate succeeds, and queues like `connected`. MACHINESCRIPTS builds it as soon as UPGRADE's Migrate ends with a hook the run engine can hear. Starting agents from sync rules needs nothing new, since D682's `start` step does it.

## D690: The app's real-SSH end-to-end test is `desktop/tests/live_flows/real_vm.rs`, run by the manual `//apps/humata2/desktop:live_flows_ssh_test` (SSHE2E manager pick, 2026-11-16; flows/cases/real/02 and 03). Decided (owner via Fable, 2026-11-16)
It lives in the live_flows crate to drive the same headless app harness (App, live clients, the typing helpers). The new target is a copy of `live_flows_test` with the real `//apps/humata2/daemon/humatad:tools` folder added and `args = ["real_vm::", "--test-threads=1"]`. Its tests skip with a message when TERMINAL_LAB_SSH_CONFIG is unset, the way the terminal lab and host SSH tests are gated. In `live_flows_test` they always skip. Waits are on observed state only.
- **Reverse:** a crate of its own once the harness moves to a shared support library.

## D691: Each real-VM run makes its own throwaway Unix user on the shared VM, with its home on a sparse 3 GiB ext4 loop image under /var/tmp/h2e-<user>, and removes it at the end (SSHE2E manager pick, 2026-11-16). Decided (owner via Fable, 2026-11-16)
The Add machine flow installs into `$HOME/.humata2`, starts a systemd user service and stops older daemons by uid (`daemon/machines/src/service.rs`). A per-run user is therefore the only way to keep runs apart and keep the Lima user's own account clean. The sparse home lets the 2 GiB free rule (D694) pass, while the root disk pays only for what is written. The test checks for 600 MiB free first and fails with how much is free. It uses sudo on the VM. It never stops, restarts or resizes the VM. Runs older than a day that were left behind are swept at the start.
- **Reverse:** a test-only remote-home override in the provider, if the owner would rather not add users to the VM.

## D692: The real-VM link cut goes through a gate in front of the real `ssh` on the host daemon's PATH (SSHE2E manager pick, 2026-11-16). Decided (owner via Fable, 2026-11-16)
While a flag file exists, the gate exits 255, as a refused connection does. Otherwise it execs `/usr/bin/ssh` unchanged. The cut sets the flag, then kills only the link `ssh` whose `-F` names the run's own temp ssh config. With no hold, a real redial would come back before the lost state could be seen, and the test would be flaky. Lima's own `ssh` and `limactl` are never touched.
- **Reverse:** none needed. It mirrors `Devbox::cut_link`.

## D693: The real-VM test gives the host daemon `--tools` from `//apps/humata2/daemon/humatad:tools`, not the app bundle (SSHE2E manager pick, 2026-11-16). Decided (owner via Fable, 2026-11-16)
MACHINESCRIPTS' w4 owns the app bundle carrying the Linux tool sets (install by arch). The test does not wait on that work. It should switch to w4's Linux-only tools target once that lands, which builds fewer platforms.
- **Reverse:** point the target at the bundle's machine-tools folder.

## D694: Open: the shared VM's disk needs to grow to 6 GiB to pass Add machine's 2 GiB free rule (SSHE2E manager, 2026-11-16). Decided (owner via Fable, 2026-11-16)
`h2-terminal-c1` has a 4 GiB disk. `NEED_FREE_MIB` is 2 GiB (`daemon/machines/src/setup.rs:29`). The product check stays as it is (main, 2026-11-16).
Freeing what tests made (`apt-get clean`, journal vacuum) took the free space from 1304 to 1651 MiB. Adding the VM from the real app (real/01) still stops at DiskLow, 397 MiB short. The fix is to grow the VM disk to 6 GiB, which is on the owner's list through main.
Until then, the real-VM test gives its throwaway user a sparse loop-mounted home (D691). The DiskLow check runs on that home and passes. It says nothing about the root disk.
- **Reverse:** n/a (nothing changed in the product).

## D720: Which agents Humata offers is a per-kind switch in `~/.humata2/config/agents.toml` (AGENTSET manager, owner's words 2026-11-16). Provisional
`[agents.<kind>]` holds `enabled` (default true) and `install_when_missing` (default false). A custom kind may also set `install = "<command>"`. With no file, every kind is enabled and nothing is installed. The plain shell is not a switch and is always on. A bad file is logged, and the daemon treats every kind as enabled with no installs, so a typo never blocks starts. `ListAgents` lists only enabled kinds unless `include_disabled` is set, and `SetAgentEnabled` (user-only) writes the file and keeps its other lines.
- **Reverse:** drop the file. Every kind is then on, as before.

## D721: A disabled kind is refused only at new starts, and existing sessions are never refused (AGENTSET manager, 2026-11-16). Provisional
StartSession, script starts and review starts (all through `begin_session`) check the switch before they relay, so the host refuses at once. Fork with a different `--agent` and desk starts check it too. Resume, restore after a reboot, move, rebuild and attach never check it. The refusal is FailedPrecondition, and its fix names Settings › Agents › <agent>.
- **Reverse:** remove the checks.

## D722: "Install when missing" is per agent, off by default, and never installs over a CLI that is found (AGENTSET manager, owner's words 2026-11-16). Provisional
The built-in commands run with `bash -lc` on the machine. claude: `curl -fsSL https://claude.ai/install.sh | bash`. codex: `npm install -g @openai/codex`. copilot: `npm install -g @github/copilot`. opencode: `curl -fsSL https://opencode.ai/install | bash`. They run only when the CLI is not found. One install runs per kind at a time, and a second start waits for it. The deadline is 10 minutes. After the install the CLI must be found, or the install counts as failed. A failure refuses the start with the command, the end of its output, and what to do: run it yourself on that machine, or turn the option off and install it your way. Humata never upgrades third-party CLIs. Its own tools (humatad2, holder, humata, stack) are installed and upgraded as before (UPGRADE).
- **Reverse:** turn the option off. Nothing else installs agent CLIs.

## D723: A CLI is installed only by a start that needs it on that machine; the start waits for it and shows it as a step (owner's proxy, 2026-11-16). Provisional
Nothing installs when the daemon starts. The install shows as a step of that start's operation, so the person sees what it waits on.
- **Reverse:** a background install when the daemon starts (rejected: it does work nobody asked for yet).

## D724: `agents.toml` ships to machines beside `repos.toml` (RL-12, D411) (AGENTSET manager, 2026-11-16). Provisional
It goes on setup and on each link-up when the copy differs, with the secret filter applied. Each daemon reads its own copy afresh for every check, so a new copy works without a restart.
- **Reverse:** keep the file host-only, so machines start every kind.

## D725: Settings › Agents is one "Agents" card of agent switches (owner's proxy, 2026-11-16). Provisional
The card sits above Default agent, with one caption under its title and no note on each row. There is one row per agent in order (Claude Code, Codex, Copilot, OpenCode, then custom), each a name and a pill switch like frames 92/93 (`settings-agent-enabled-<id>`). Under an enabled agent with an install command there is an indented "Install when missing" row, off by default (`settings-agent-install-<id>`). It shows the exact command in mono and "Never replaces one that's there", and is hidden while the agent is off. There are no installed, missing or version chips, and no Save button. A failed save puts the switch back and shows the error. `AgentDefinition.install_command` (17) carries the command. The pickers (new session, fork, desk, channel, tab "+", PR review, submit review) and Default agent offer `SettingsState::offered_agents()`: the enabled built-ins, plus Terminal where it was offered. Before the list loads, they offer them all. A default agent that is disabled falls back to the first enabled agent.
- **Reverse:** remove the card. The pickers then offer every agent again.

## D726: Missing agents are said only where they block (AGENTSET manager, 2026-11-16; D-no-unfixable-detection). Provisional
A start whose CLI is not found is refused with what to do: install it yourself on that machine, or turn on "Install when missing". Nothing polls for missing CLIs or shows "missing" anywhere else. The machine page list of agents a machine has is left for later.
- **Reverse:** n/a (nothing extra is built).

## D727: "Found" resolves the CLI the way launch does (AGENTSET manager, 2026-11-16). Provisional
The CLI looked for is the kind's program, or the CLI a wrapper names: opencode looks for `opencode`, and `/usr/bin/env <cli>` looks for `<cli>`. An absolute path must be an executable file. A bare name must resolve on the machine's PATH the same way the session's launch resolves it, with a 10 s deadline. The worker records the exact rule in `agent_gate.rs`.
- **Reverse:** n/a.

## D728: The switches are two humatad modules, not a new crate (AGENTSET manager, 2026-11-16). Provisional
`agent_switches.rs` (the file) and `agent_gate.rs` (the checks and installs) are used only by humatad. A crate would add a workspace member and a repin for no other user.
- **Reverse:** move them to a crate once a second user (the CLI, say) needs them.

## D729: The frame "Settings › Agents with agent switches" goes to the owner's design wave as one light frame of all three states (owner's proxy, 2026-11-16). Provisional
The frame shows an agent that is on, one that is on with "Install when missing" on, and one that is off. It is rendered headless to the scratchpad, never into the goldens. A golden is recorded only after the owner approves the frame.
- **Reverse:** n/a.

- **Ruling (owner via Fable, 2026-11-16):** Answered. Grow `h2-terminal-c1` to 6 GiB. Main does it at a moment when no terminal lab session is live on the VM, because the resize stops it. The 2 GiB product rule stays, and D691's per-run sparse home stays so runs remain apart.

## D730: A plan never asks; the call that acts does, and approving it runs the action (Fable 5.2, 2026-11-17). Decided (by Fable)
PlanArchive, PlanDelete, PlanPush and PlanRemoveChannelMachine check the access rule but never put a request in the Inbox. Never still refuses the plan. Ask answers the plan as Allow does. Only the executing call (ArchiveSession, DeleteSession, PushAndOpenPullRequests, RemoveChannelMachine) asks, so approving its Inbox request carries out the action, and refusing it does nothing. Before this, the plan asked and approval only re-ran the plan, so an MCP agent's `session_archive` or `session_delete` never acted, and the CLI asked twice.
- **Why:** a plan only reads, and the user's yes is meant for the action. One request per action, whose answer is the action's result.
- **Follow-ups (Fable's refinements, not done here):** show the plan on the act's Inbox card; when a destructive choice's plan token has expired by approval time, re-plan and run only if the fresh plan matches what the card showed, else fail as plan_stale.
- **Reverse:** gate the plans with `answer_gated` again in `session_service.rs` and `channel_service.rs`.

## D731: A Desk start the host asked a box for survives the host dying before the answer (bug fix, 2026-11-17; follows D432). Decided
The host keeps a journal of each Desk start it asks another machine for, in `state/desk_remote_start/<session>.json`, written durably before its own records, and saves its record as started (`StartStep::Done`) from the first write. A host restart's session recovery therefore never undoes the start as one cut short on the host. Instead, the restart restores the start's operation under its own id, waits for the machine's link, and asks the machine again under the same session id. The machine answers with the session it kept (`known_start`), or starts it. A refusal removes only the host's records and never a folder. The journal goes when the machine answers or refuses.
- **Why:** before this, the host's record said `Recorded` until the answer arrived. A restart in that window found no "desk" channel, failed to finish the start, and undid it, so the host lost the session the box kept running.
- **Not changed:** a link that breaks during the live call still ends the start with an error and removes the host's records, as before. Following a lost answer there, as `start_session_answered` does for channel sessions, is a follow-up.
- **Reverse:** remove `desk_service/remote_asked.rs` and its calls in `desk_service/remote.rs` and `desk_service.rs`, and save the host's detail as `Recorded` again.

## D732: A pull request draft fits lines in the same repository key, or in the bare name equal to the PR repo's name part (bug fix, 2026-11-17; follows D624). Decided
Lines in `api` (a box's clone with no owner) can carry a draft for `acme/api`. Lines in `acme/api` can too. Lines in `other/api` or `web` cannot. D624 keys a repo as `owner/name`, or as the bare name when there is no owner, so a bare name means the same repository under its short key. The one rule is `humata2_clients::comments::repo_holds_pull`. Draft for GitHub (`github_target`) and the fake comments client use it, and the daemon's CreateComment check repeats it in `review/comments.rs`.
- **Why:** `github_target` already offered Draft for GitHub for a bare name, but the fake and the daemon refused the draft unless the keys matched exactly. The button then led to a refusal.
- **Reverse:** require the exact key everywhere, offer the button only on an exact match, and make the fixtures' repos owner-qualified.

## D733: A working report restarts the 10 s quiet net (bug fix, 2026-11-17; reads D-turns' "WORKING, until 10 s with no output"). Decided
When a hook newly says working, the session shows Working, and the 10 s of quiet that turns it Probably idle are counted from that report. Quiet from before the report no longer counts. Before this change, an agent quiet for more than 10 s that then reported working (`humata turn working`, or UserPromptSubmit after a long pause) went straight to Probably idle, so a "while the session works" view such as SE-24's "n others can type here" never showed. A repeat of the same report is still a no-op. In the code, `turns::set_hook` resets the entry's quiet and counts the report in `armed`, and the output watch (`watch.rs`) restarts its `Quieting` clock when that count moves.
- **Reverse:** drop `armed` and `Quieting::rearm`, and keep counting the quiet from the last output only.

## D734: The unkept Desk folder chip reads "Goes 7 days after close" (2026-11-17; reads DK-3, DV-4). Decided
The chip used to read "Not kept · goes 7 days after close". In the 638 pt session pane of frames 37 and 90 that pushed **Keep folder** off the bar's right edge, and the golden cut the chip to "Not kept · goes 7 days aft…". **Keep folder** sits beside the chip and already says the folder isn't kept, so the chip now gives only the deadline. Fable judged this; the golden code, `ui/src/desk_bar.rs`, SPEC DK-3 and the flow docs use the new wording.
- **Reverse:** restore the longer wording and let the folder name shrink instead.

## D735: Native captures 67, 68 and 69 re-recorded after the review diff's number column and the Context box's padding moved to the goldens (Fable 5.2, 2026-11-17). Decided (by Fable)
`look_test` compares the recorded captures in `__snapshots__/native/`, so the two code changes made after Fable's earlier approval (run 41) only count once the captures are re-recorded. The review diff's number column now fits 36 pt in `ui/src/panel_changes.rs` `review_code`, and the Review a pull request dialog's Context box has 10 pt top padding in `ui/src/overlay_pr_review.rs`. Re-recorded with `native_test_record -- _frame_6`; only 67, 68 and 69 changed. Look scores against the goldens: 67 0.209% → 0.007%, 68 0.947% → 0.066%, 69 0.011% → 0.011%. Fable approved all three: numbers and code land on the golden's x positions, nothing overlaps or clips, and the Context text sits at the golden's height.
- **Follow-up, done (Fable approved, 2026-11-17):** in frame 68 the +/- sign sat tight against the line number (`139−`), where the golden leaves about one cell (`139 −`). In a review's inline code, `code_rows_focused` in `ui/src/panel_changes.rs` now draws the sign one cell right (a relative offset, so no column moves) when the line's first character is indentation, which puts it in the code's first, blank cell as the golden does. A line that starts at column 0 keeps the sign in its own cell so it never covers code. Native 68 was re-recorded (only the signs changed; look 0.066% → 0.049%). Native 69 stayed within its record tolerance, because its signs are behind the dialog. Fable's note: in a hunk that mixes indented and column-0 lines the signs sit one cell apart between rows, which is better than moving a column; judge it when a capture first shows such a line.
- **Reverse:** drop the `.when(review && …starts_with…)` offset on the sign, and restore the previous captures from the commit before this one.

## D736: A box review's diff is forwarded to the box; Mark reviewed, review plans and note re-anchoring follow separately (Fable 5.2, 2026-11-17; reads PR-4, PR-21, RV-1, RV-6). Decided (by Fable)
For a review session on another machine, the host checks access first. It then resolves each repo's pull request base through the link (RunReviewGit, as the heads check does) and forwards GetDiff/GetFileDiff to the box's daemon with that base written out as an explicit merge-base spec (`wire::with_pull_request_bases`). A down link answers machine-offline. For these sessions the host runs none of a local GetDiff's follow-ups: it records no shown trees and re-anchors no line notes or non-PR comments. Everything else that reads the session's clones (MarkReviewed, review plans, line notes) answers Unimplemented: "this session runs on <machine>, and Mark reviewed, review plans and line notes don't work yet for a session on another machine". It no longer fails on the host's empty placement of the folder. PR review comments go through the host's `gh` and are unaffected.
- **Why:** forwarding MarkReviewed without the shown-tree baseline would mark against the wrong trees, so it needs its own design rather than riding on the transport fix. Queued as the next change.
- **Reverse:** forward MarkReviewed and the review-plan calls to the box, have the forwarded GetDiff return the shown trees for the host to record, then drop the refusal in `review.rs` `not_here`.

## D737: Design goldens 37 and 90 and native captures 37 and 40 re-recorded for the hidden-path Desk folder bar, D734's chip and DK-14's new desk session dialog (Fable 5.2, 2026-11-17; reads D460, D466, D734, DK-14). Decided (by Fable)
Design goldens 37 and 90 still showed the old Desk folder bar, with the path `~/Humata/desk/plan-q4` and a "scratch" chip. They now show the bar that `design/desk.rs` `folder_bar` draws: Desk folder, Copy path, the chip "Goes 7 days after close", the machine, + Add repo, and Keep folder. Native capture 37 changes only by that chip, in the picture and in its `view.txt` Status line. Native capture 40 now shows the DK-14 dialog with the Agent access labels kept to one line (`ui/src/overlay_desk_new.rs`). Re-recorded with `design_test_record` and `native_test_record -- desk::`. That run also rewrote design 39, 42, 43, 55, 79, 80 and 89, which record at exact tolerance; those changes are rendering noise within the verify tolerance, so they were put back unchanged. Look scores against the goldens: 37 0.694% → 0.063%, 40 1.199% → 0.000%. Fable approved all four: the bar matches D460/D466 and D734, nothing is clipped, and nothing else changed.
- **Follow-ups (Fable, not blocking):** the mock terminal's first line still shows `~/Humata/desk/plan-q4 $`, so a neutral prompt such as `plan-q4 $` would keep the path off screen; native 37's board panel expand and close icons sit 3–9 pt left of the golden's, and the rail's Inbox badge is a point or two off; in the native sidebar the selected desk row is semibold and the channel counts are regular, where the golden has them the other way round.
- **Reverse:** restore the previous four snapshots from the commit before this one.

## D738: Humata sets no shell prompt; its mock terminals show `plan-q4 $`, not a path (Fable 5.2, 2026-11-17; narrows D466; reads D460, D737). Decided (by Fable)
D466 covers Humata-authored chrome only. Terminal content (the user's prompt, agent output) belongs to the user and their tools and is never rewritten. Humata sets no PS1 or PROMPT: `daemon/holder/src/server.rs` sets only TERM, COLORTERM and LANG. A ZDOTDIR shim or PS1 override would override the user's config without opt-in, break real setups (p10k, starship, fish), and still leave paths in `pwd` and agent output. Humata-authored mocks, fixtures and goldens follow D466 and show the session name, not a path. The first mock terminal line `~/Humata/desk/plan-q4 $ claude` is now `plan-q4 $ claude` in `clients/src/fixture/desk.rs`, `desktop/tests/design/desk.rs` and `desktop/tests/design/roam.rs`. Re-recorded: design 37, 89, 90 and 97 (`design_test_record`), and native 37 and its view.txt (`native_test_record -- desk::`). Design 39, 42, 43, 55, 79 and 80, which the exact-tolerance record rewrote with noise, were put back unchanged. Design 89 also picks up the Desk folder strip that D737 gave 37 and 90; its old golden still drew the path behind the dialog. The look score of native 37 against design 37 stays at 0.063%. Fable approved all five frames: only the first terminal line changed, with no clipping or reflow.
- **Follow-ups (Fable, not blocking):**
  - Sweep the other dialog-over-board goldens for a stale path strip.
  - Native terminal text runs slightly narrower than the design's: line ends at about 920 px against 934 px. This is inside the masked terminal body.
- **Reverse:** restore the three mock lines and the six snapshots.

## D739: The Diff panel says "against X" once while the repos share a base; Add repo… is quiet text; the Journal drawing follows VW-10; the session menu hides Stop on a stopped session (Fable 5.2, 2026-11-17; amends D510; reads RV-2, CH-21, VW-10, SE-7, D295). Decided (by Fable)
Fable's notes on native 17's Diff panel, taken together:
- **Repo base chip (pick A of four):** while every repository shares the header's base, each repo's second line shows only the base menu's chevron (a 24 pt target named "<repo>: against <base>", opening the same per-repo menu) beside Push & PR. When the bases differ, every line shows "against X" as golden 85 draws it. B (hide the chip, add "Pick per repo…" to the header menu), C (hide it for a single repo) and D (keep D510 as it was) were turned down: B and C need header-menu work, D printed "against main" three times.
- **Add repo… (CH-21)** is a quiet accent text action like Push & PR, not a bordered button, so the summary line keeps the goldens' height. Golden 85 draws none.
- **RV-2** now says **Push & PR**, as D293, D388 and golden 85 do. Wording only.
- **Design 08** is redrawn for VW-10: newest first; title and state, then kind · the fork it came from · when, then the summary, then Open. The design diff panel (session.rs, frames 01, 02, 11, 15, 16, 17, 30, 62, 65) draws the chevron and Push & PR line.
- **Session ⋯ menu:** a stopped session's menu has no "Stop the agent" (it was greyed out); "New desk session for this session…" gains its ellipsis, and the menu card widens from 230 to 256 pt so that label fits.
- **Re-recorded:** design 01, 02, 08, 11, 15, 16, 17, 30, 62, 65; native 01, 02, 11, 15, 16, 17, 30, archive-unpushed, changes-pushed, changes-wrapped-long-line (light, dark), fork-dialog, rename-dialog, session-menu-closed, session-menu-open, session-menu-stopped-dark; panels changes and changes-full (light, dark). Fable approved all; it rejected the first menu captures for a clipped label, fixed by the wider card as it prescribed.
- **Follow-ups (Fable, not blocking):** the native Journal lists the fake's seeded views oldest first (the fake seeds them in reverse) and says "Kept", which is not a VW-10 word; with a pull request open, the api row's "Push & PR" turns primary text and "PR #1" overhangs the other rows' right edge by about 7 pt; give the lone chevron a hover tooltip and a visible hover/focus state; the session menu draws its items as bordered centred buttons rather than menu rows.
- **Handed to VERIFY:** the native Journal's order (the fake seeds its views oldest first) and its "Kept" wording belong to the VERIFY lane's fixture work; this decision does not change them.
- **Reverse:** restore the listed snapshots from the commit before this one and put back the "against X" chip on every repo line, the bordered Add repo… and the greyed Stop.

## D740: The Diff panel's quiet actions keep their colour on hover; the lone chevron is a real button; the session menu draws menu rows; the machine page's native and design frames follow frame 57 and the read-only rule (Fable 5.2, 2026-11-17; D739 follow-ups; reads D510, D739). Decided (by Fable)
- **Quiet actions** (`ui/src/panel_changes.rs`): Push & PR and Add repo… use a custom button variant whose text stays accent and whose hover and press fill `look.fill`. The ghost variant forced the text to `accent_foreground` on hover, which turned Push & PR primary text in changes-pushed.
- **PR #1:** the head row's second line is inset 8 pt rather than 4, so the pull request's number ends where the totals above it do instead of overhanging by about 7 pt. The design `repo_actions` still insets 4 pt; no look frame covers the Diff panel, so it is left as drawn.
- **Lone chevron:** while the bases agree, the chip is a 24×20 button with the same quiet variant, a "<repo>: against <base> · change" tooltip, hover and focus states, and the same per-repo menu. Residual: gpui-component's Button exposes no aria-expanded, so the chevron-only chip no longer reports collapsed; the worded chip still does.
- **Menu rows** (`ui/src/action_menu.rs`): the shared menu component gains `item`, a borderless, left-aligned 26 pt row with a hover fill. The session ⋯ menu uses it; the machine page's ⋯ menu still draws `ghost_in` buttons and can move to rows later.
- **Machine page order:** `ui/src/hosts.rs` already puts "Free space…" before "Open a shell", as frame 57 and design `pages.rs` do; native 28-hosts, 56-add-machine and 58-add-machine-unattended were stale and are re-recorded.
- **Design 79 and 80:** native dims a read-only machine's variable links (`ui/src/machine_page.rs`, "A read-only machine takes no variable changes"); the design `Machine.read_only` now draws them tertiary too, so design 79-machine-migrating and 80-machine-read-only are re-recorded. Every other design frame from that record was put back.
- **Re-recorded:** native 01, 02, 11, 15, 16, 17, 30, archive-unpushed, changes-pushed, changes-wrapped-long-line (light, dark), fork-dialog, rename-dialog, session-menu-closed, session-menu-open, session-menu-stopped-dark, 28-hosts, 56-add-machine, 58-add-machine-unattended; panels changes (light, dark) and changes-full (light, dark); design 79-machine-migrating, 80-machine-read-only.
- **Verdicts:** Fable approved every frame listed. Its non-blocking follow-ups: end Push & PR on the totals edge even with no PR number after it (its trailing padding now leaves it about 7 pt short); put back aria-expanded on the chevron-only chip once Button carries it; move design `session.rs` to the 8 pt inset and add a look frame over the Diff push row; set Archive session… (and the Stop / Push group) apart with separators; make dimmed variable links inert with a "Read-only until migrated" tooltip; wrap design 57's explanation and warning lines inside the dialog. The header icon shift it saw is the other D739's panel header change.
- **Reverse:** revert `panel_changes.rs`, `action_menu.rs` and the session menu in `pages.rs`, and restore the listed snapshots.

## D742: The panel header's buttons, the rail's Inbox badge and the sidebar's weights follow the goldens (Fable 5.2, 2026-11-17; D737 follow-ups; narrows L17.2). Decided (by Fable)
- **Panel header** (`ui/src/panels.rs`): the 16 pt expand and close buttons now sit in a row with a 6 pt gap and a -2 pt right margin. Their 12 pt glyphs land where `design/session.rs` `panel_header` draws them: 10 pt apart and 14 pt from the edge. Before, they were 2 and 6 pt too far left.
- **Rail Inbox badge:** matches `kit::badge`: px 6, no minimum width, 10.5 pt text, and `look.accent` rather than `look.selection` (dark 0x3b8cff; light is unchanged).
- **Sidebar weights** (`ui/src/sidebar.rs`, `ui/src/sidebar_desk.rs`): a folded channel's rollup count takes the row's weight, BOLD when unread. L17.2 is narrowed: selection alone never bolds a sidebar session row.
  - Channel rows are semibold when unread, or when selected with requests waiting (goldens 15, 16).
  - Desk rows are semibold only when unread (golden 37). They show no waiting count, so a waiting request doesn't bold them. If a later golden gives Desk rows a count, they take the channel rule.
- **Re-recorded** with `native_test_record -- desk::`: native 36, 37, 38, 40 and 55. Look scores: 36 0.008%, 37 0.000%, 38 0.000%, 40 0.000%, 55 0.054%. `panels_test_record` kept every panels frame.
- **Design 55:** re-recorded for the desk drawing that already matches native ("your folder", "Kept").
- **Design 39, 42 and 43:** an uncommitted edit had shortened the version bar's placeholder in `design/desk.rs` to "What changed". Fable rejected that because native and flows say "What changed? (optional)", so the text was restored and these three keep their earlier goldens. Design 41, recorded earlier today from the shortened string, is re-recorded with the restored placeholder (clipped in the narrow pane); Fable approved it, and native 41 scores 0.170% against it.
- **Verdicts:** Fable approved native 36, 37, 38, 40 and 55 and design 55, and re-approved design 37 and 90 and native 37 and 40 as the desktop agent handed them back.
- **Follow-ups (Fable, not blocking):** a clipped placeholder should end in an ellipsis in design and native, then re-record 41. In native 37 the terminal mock's prompt box sits about 14 pt higher than the golden's, has no right edge, and shows ">" with no space before the cursor (masked).
- **Reverse:** revert the three ui files and restore the snapshots listed.

## D741: A fork or move copy leaves out what git or the agent drops mid-copy; opening or closing a shell tab pushes its session (live flows fixes, 2026-11-17). Decided
- **Copy (`daemon/sessions/src/tree_copy.rs`):** an entry gone since its folder was listed is left out instead of failing the fork or move, and git's own `*.lock` files under `.git` are never copied, so the copy's clone never looks locked; any other copy error still fails and names the file. Reverse: drop `left_out_if_gone` and `git_lock`.
- **Tabs (`daemon/humatad/src/terminal.rs`, `clients/live/src/terminals.rs`):** opening or closing a session's shell tab publishes that session's Updated event, and the client keeps its last tab list when re-listing fails rather than showing none. Reverse: remove the `tell.session` calls and `listed_tabs`.

## D743: Humata never tells an agent where its repos are; the person prompts the agent, and an agent that wants its session's repos asks a tool (owner, 2026-11-16; recorded 2026-11-17; amends CH-21). Decided (by owner)
- **Rule:** no message, MCP initialize instruction, tool description, environment variable or first-message line names a clone's path or branch. Adding a repo to a session (CH-21) clones it and adds its group to the diff, files and comments, and says nothing to the agent.
- **What agents may get:** with their kind's switch on in Settings › Agents, a session-scoped humata MCP server and Humata's tools on PATH, whose descriptions say only what the tools do. `humata whoami` and the MCP tools return the session's repos when the agent asks.
- **Why:** the owner prompts the TUIs; pushed hints surprise the person and the agent alike.
- **Reverse:** a new owner direction only.

## D744: A machine session's permission card is journaled to the host's Inbox when raised (remote Copilot fix, 2026-11-17)
- **Problem:** D191 left open how a remote machine's agent hook reaches the person: the card was raised in the machine's own Inbox and never saved, so D225's journal never carried it. The host's Inbox stayed empty and the hook held for its full hour. That broke the live remote Copilot flow.
- **Pick:** on a role-machine daemon, `raise_permission` saves the card and journals it like every other new request (D225). The host mirrors it (D226), and its answer goes down the link to the waiting hook. The hook letting go withdraws the card, and that withdrawal is journaled too. The stored `InboxEntry` gets `permission = 6`, so the machine's next start withdraws a card still pending (CALL_ENDED), journaled, so no card is left pending after a restart (D191.4). On the host nothing changes: cards are still not saved when raised. No login or bind is involved: the hook uses its session token on its own machine, and the host relays the answer as its own user.
- **Upgrade (D-UPGRADE):** additive, no Migrate item. An entry kept without the field decodes as `permission = false`, and false is right for every old entry. Before this change a permission card was saved only once settled, and the flag is read only for a pending entry, so no old pending entry was ever a permission card. Machine journal entries carry `humata.v1.InboxRequest`, not the stored `InboxEntry`, so they don't change. The test `a_pending_request_kept_before_permission_cards_were_saved_restores_as_before` (inbox/kept.rs) decodes an entry encoded without the field and checks it restores as before. An older machine daemon still raises cards it never journals, but the force-upgrade to the new release fixes that.
- **Reverse:** drop the commit in `raise_permission` and the `permission` field; remote hooks again get no decision.

## D745: Synced skills reach a machine's agents as links in each agent's own skills folder (VF-5, Fable's ruling, 2026-11-17). Decided (by Fable)
- **Problem:** a sync put the set in `~/Humata/.skills/<agent>/` and nothing pointed the agents at it, so a remote Copilot listed none of the synced skills (`flows/cases/results/copilot-sync.md` G2). SK-4 says agents started there are pointed at the managed folder.
- **Pick:** no home variable (`COPILOT_HOME`, `CODEX_HOME`, `CLAUDE_CONFIG_DIR`; D193). After each applied sync the machine links every synced skill into the agent's own folder, `~/.copilot/skills`, `~/.claude/skills` or `~/.codex/skills` (each CLI's personal skills folder, checked against `copilot help skill` 1.0.113 and codex 0.169), as `<name>` → `~/Humata/.skills/<agent>/<name>`. That is an absolute link through the stable `.skills` link, so SK-9 holds. An agent folder that isn't there is made only when that agent has a synced skill. Humata's links are the symlinks pointing into `~/Humata/.skills`; anything else of that name (a folder, a file, someone else's link) is the machine's own and is never replaced or removed (SK-4). Humata's links whose skill left the set are removed, and so is one whose clash is picked `local` in `~/Humata/.skills-choices.json` (SK-5). A `synced` pick still can't displace the machine's own folder: the person moves theirs away. Humata's own links no longer count as clashes, so a second sync doesn't ask again.
- **A linking failure** after the set switched fails the sync (`Internal`, "the agents don't see the new skills yet"), so the host keeps its old record and the next sync links again.
- **Open:** the host keeps the clash picks in its record (`skills_service.rs`), and the link carries no picks, so the machine's `.skills-choices.json` is empty today and a clash is always the machine's own. Sending the picks with the set is a proto change for its own lane.
- **Reverse:** drop `Managed::link_agents` and its call in `peer_skills::apply_inner`; the set stays where it is and agents see none of it.

## D746: Copilot 1.0.113-1's new records are known to the strict reader, and a landing runs that strict read before it adds any repo (VERIFY, 2026-11-17; reads D736, desk.proto). Decided
- **Known now:** `instruction.update_delivery`, eight `model.*` record kinds, workspace.yaml's `remote_steerable` and `mc_*` keys (with `mc_pending_repository_warning`) and `tool.execution_complete.fileEdits`, each with exactly the fields seen in scrubbed real captures (`daemon/conversation/testdata/copilot/1.0.113-1/`). Anything else is still refused.
- **Order (`daemon/sessions/src/carry_landing.rs`):** `check_landing` runs the adapter's strict check before `add_repos`, so a refused Desk move or carry changes nothing.
- **Still refused:** `system.notification` (background shells); a session that ran one forks code-only until a separate ruling.
- **Amended (VF-60, 2026-11-17):** `system.notification` is now known when its `kind.type` is `shell_completed` or `shell_detached_completed` (the second has no `exitCode`), with exactly the fields seen in real captures (`NOTIFICATION_KINDS` in `copilot.rs`). A session that ran a background shell forks with its conversation. Any other notification kind is still refused, and that session still forks code-only.
- **Reverse:** drop the allowlisted kinds from `copilot.rs` and the fixtures.

## D748: Agents get Humata's MCP server, tools on PATH and pre-allowed tools per session, through flags only (VF-1, VF-2, VF-42; follows D743, 2026-11-17). Decided (by Fable, amended by owner via D743)
- **Problem:** no code gave a session's agent Humata's `humata` MCP server, `humata` wasn't on a tab's PATH, and MCP.md §1 described a holder-written config that didn't exist.
- **Pick:** at its start the daemon writes `state/humata-mcp-claude.json` and `state/humata-mcp-copilot.json`. Each names `humatad2 mcp --socket <socket>` and holds no token: the server reads `HUMATA_SESSION_TOKEN` from the agent's environment, and the daemon still checks the process tree. Each built-in kind is started and resumed with flags confirmed against its CLI's `--help`. Nothing goes into the agent's user config, the repo, AGENTS.md or CLAUDE.md, and nothing is typed into the TUI.
  - Claude Code 2.1.326: `--mcp-config=<file>` and `--allowedTools=mcp__humata,Bash(humata:*)`, never `--strict-mcp-config`.
  - Copilot 1.0.113: `--additional-mcp-config=@<file>`, `--allow-tool=humata` and `--allow-tool=shell(humata:*)`.
  - Codex 0.169.0: `-c mcp_servers.humata={ command, args, env_vars = ["HUMATA_SESSION_TOKEN"], default_tools_approval_mode = "approve" }`.
  - The variadic flags take their value after `=`.
- **Shells and custom kinds:** every session tab, plain shells included, gets `PATH` with the `humata` binary's folder first, plus the token and socket. An agents-file kind gets the server only where its args name `{humata_mcp}`, the Claude-format file.
- **Saved agents:** at start, saved agent launches take this daemon's tools arguments (`Agents::with_tools_in`), so an agent started before the change resumes with them. Rebuilds and forks plan anew.
- **The switch:** a kind turned off in Settings › Agents isn't started (D720), so its tools never run.
- **D743:** the MCP initialize instructions say what the tools do and name no path, clone, folder or branch. `humata whoami` (agent tier) returns `folder` and `clones` (repo, path, branch) when asked, and the `session_info` tool returns the repos.
- **Open:**
  - Copilot shows a non-allowlisted server's initialize instructions only with `--allow-all-mcp-server-instructions`. That flag isn't passed, because it would also cover the person's other servers.
  - Codex has no per-command allow flag for `humata`.
- **Reverse:** drop `with_humata_tools` and the `install_mcp_configs` call in `serve.rs`, and the PATH in `identity::tab_env`.

## D747: The Markdown gallery's pixel goldens draw code in the theme's mono face, and the pixel tests run in their own binary (Fable 5.2, 2026-11-17). Decided (by Fable)
- **Cause:** gpui-component 0.9.1 resolves its default mono family once per process (`theme/mono_font.rs` OnceLock). `markdown/headless/tests/snapshots.rs` mixed font-less `#[gpui_kit::test]` contexts with headless Metal renders, so whichever test ran first fixed code to the system UI face or to Menlo. The old goldens came from the font-less order.
- **Change:** the three Metal tests moved to `tests/pixels.rs` (target `:pixels`, goldens in `tests/__snapshots__/pixels/`). `older_gallery` and `annotate` moved to the headless library for both files. gallery.png and gallery-annotated.png re-recorded in Menlo, where the box diagram lines up.
- **Reverse:** merge the files back, and expect order-dependent fonts again.

## D749: The versions bar's placeholder ends in an ellipsis, its step buttons are 26 pt, and the mock agent's cursor sits after "> " (Fable 5.2, 2026-11-17; D739 follow-ups). Decided (by Fable)
- **Placeholder:** in a narrow pane the Save version box's placeholder ends in "…" rather than being cut mid-word (frame 41).
  - Design `versions_bar` puts the text in a truncating child.
  - Native `ui/src/doc_history.rs`: the input can't ellipsize its own placeholder. While the box is empty and unfocused, the same words are drawn over it with `truncate()`, and the input keeps its placeholder for screen readers.
- **Step buttons:** the native < and > buttons are 26 pt wide, as drawn, instead of being label-padded to about 38 pt. The note field now starts where the design's does.
- **Mock agent cursor:** the mock agent's input box (`clients/src/fixture/terminal.rs`) puts the cursor at column 5, after "│ > ". That gives "> █" as in design `session.rs` `input_box`. This shared fixture moves the cursor one cell in every agent frame, about 1,000 px, which is within native tolerance. Only my frames were re-recorded.
- **Re-recorded:**
  - Native 37, 39, 41, 42 and 43, with `native_test_record -- desk::` and `-- document::`.
  - Design 41.
- **Look scores (budget 0.5%):** 37 0.000%, 39 0.021% (was 0.140%), 41 0.006% (was 0.170%), 42 0.014% (was 0.127%), 43 0.007% (was 0.149%).
- **Verdicts:** Fable approved all six frames.
- **Open, for the terminal owner:**
  - When a side panel narrows the pane, the agent grid keeps its first size (135 columns), so the box's right border is off-screen (native 07, 37 and others). `FitPolicy::Shown` refits only while focused and in control.
  - The box's height and offset are D219's cell-metric gap; remove the TERMINAL_BODY mask once that is fixed.
- **Reverse:** restore the field and step buttons in `doc_history.rs`, `\x1b[4G` in the fixture and the design field, then re-record the frames listed.

## D750: A shown terminal fits its own new bounds while unfocused, and never re-asks for a size it already asked for (terminal owner, 2026-11-17; D749's open item, reads D271 point 3). Provisional
- **Cause:** `FitPolicy::Shown` (`terminal/client/src/replica.rs`, `ReplicaPty::resize`) sent a Resize only while focused or before its first size. The view still recorded the unsent size as asked for, so when a side panel narrowed an unfocused pane the grid kept its first size (135 columns) and drew past the pane's right edge (native 07, 08, 37, 68).
- **Pick:** under `Shown`, the replica remembers the columns, rows and cell size it last sent. Unfocused, it sends a Resize only when its own bounds fit a different size from that. Focused, it fits as before. A Resized or new snapshot from another view's resize makes it fit the same bounds again, which match what it last sent, so nothing is sent: the view that set the size keeps it, and two views never take turns (D271 point 3 holds). A failed send is not remembered, so the next fit retries. Holders are untouched; this only changes when a Resize frame is sent. No debounce: the view already drops fits that give the same cell size, so a divider drag sends one Resize per column change, as a focused view already does.
- **Tests:** `an_unfocused_shown_replica_refits_when_its_own_bounds_change` (failed before the fix: 100 columns stayed 100) and `an_unfocused_shown_replica_never_takes_back_a_size_another_view_set`, in `terminal/client/tests/component.rs`.
- **Follow-up:** the `FIT` comment in `ui/src/terminals.rs` still describes the old rule; native goldens 07, 08, 37 and 68 need re-recording.
- **Reverse:** restore the `sized` flag in `ReplicaPty::resize`.

## D751: D743 amends PR-5 and SL-4's note: the review chat's first message names each PR's URL, title, author and size, not its branches or clone location, and the rebuild note drops the branch name (daemon owner, 2026-11-17). Reversible
- **Pick:** `first_message_of` in `apps/humata2/daemon/humatad/src/review/pull_requests/start.rs` lists each PR as URL, title, author, files and lines. The base and head branches and the "cloned at its head branch in this folder" text are gone; the agent gets branches from session_info and review_context. `note_of` in `apps/humata2/daemon/humatad/src/session_service/rebuild.rs` says "owner/name lost ..." with no branch.
- **SPEC:** PR-5 is already updated by main: the agent is told the URL, title, user context and PR-23 tools, and asks session_info or review_context for branches or clone locations.
- **Tests:** `the_review_chat_is_told_each_pr_and_the_users_context` in `apps/humata2/daemon/humatad/tests/pr_review.rs` (no branch names in the first message); `rebuild_on_box_2_clones_last_pushed_sha_resumes_mirrored_conversation_and_names_lost_work` in `apps/humata2/daemon/humatad/tests/rebuild_session.rs` (the agent's terminal does not name the branch).
- **Impact:** string only; no schema or upgrade impact.
- **Reverse:** put the branches back into both message formats and the test assertions.

## D752: An add on This Mac waits behind This Mac's daemon Migrate item, so a first-seen host key never meets a client at another release (host_key_unknown follow-up, 2026-11-17; UP-1, D281, D290 m, D370, D374). Decided
- **Problem:** errors.proto kind 59 `host_key_unknown` replaces the old first-seen answer (`host_key_changed` with an empty expected fingerprint), and `clients/live/src/machines.rs` now shows the Confirm card only on `host_key_unknown`. A CLI at another release is already refused everything but ServerInfo and Upgrade (`daemon/humatad/src/upgrade/gate.rs`, `cli_ready` and `cli_check`; D281 e, D290 m), so an old `humata hosts add` can't reach a new daemon. The desktop is not held to the release (D290 m): between installing a new Humata and clicking Migrate, the app talks to This Mac's daemon at the old release, and the Add machine dialog ran the add anyway. The new app then got the old answer and showed a failure where it should offer Confirm. In the other direction, an app older than the daemon got kind 59, which it can't read. Both are mixed-version fallbacks.
- **Pick:** the Add machine dialog already gets This Mac's Daemon Migrate items (`features/app/src/lib.rs`, the `add` block; D370's host item and D374's app-held item both arrive). `AddMachineAction::Start` (`features/hosts/src/add.rs`) now runs nothing while one of those is unanswered. It shows the D370 "This Mac needs migrating" card with Migrate This Mac, or Migrating when the item already migrates. Once the item is migrated, the existing rerun starts the same add. No new Migrate item, proto field or daemon gate is needed: the daemon already posts the item, and the app is the only client that needed holding. From this release on, an older app is held too, because the newer daemon posts "Migrate the daemon on This Mac" for the release the older app staged.
- **Upgrade (D-UPGRADE):** kind 59 is an additive oneof case. Nothing is stored, journaled or sent between machines with it: it travels only from This Mac's daemon to this Mac's app or CLI. The CLI is held by the bundle header, and the app now by this pick. Remote machines' daemons never send it, since the host runs the SSH connect. Not covered: an app built before this change, opened after a newer daemon was installed. That is a downgrade (D281 h), and its dialog shows the daemon's words as a failure, not a Confirm card.
- **Tests:** in `features/hosts/tests/migrate.rs`, `an_add_while_this_macs_daemon_waits_to_migrate_does_not_run_and_offers_migrate`, `an_add_held_behind_this_macs_migrate_runs_once_it_is_migrated` and `an_add_while_this_mac_is_migrating_waits_and_runs_once_it_is_migrated` failed before the fix. `an_add_after_this_mac_was_migrated_runs_at_once` and `a_store_item_waiting_does_not_hold_an_add` guard against holding too much.
- **Reverse:** drop `this_mac_unmigrated` and its check in `Start`.

## D753: Text is never typed into a holder from before the wait for keys; it is refused with the "Migrate the holders" item named (BUG-T1 follow-up, 2026-11-17; force-upgrade, D373, D378). Decided
- **Pick:** BUG-T1's `AwaitKeys` frame is new, and an older holder hangs up on it. `humata2_holder::await_keys` no longer answers `Unsupported` for that; it fails with `HolderError::Outdated`, and `ReadsKeys::Unsupported` is gone. `type_then_return` (first message, notes, the interrupted note, rebuild note) and the start-fresh seed fail with it. TerminalService.Type and the review comment paste refuse with FailedPrecondition before anything is typed, and a Type key is released, so a retry after the migrate types once. The message says the holder is from an older build; the fix says to click Migrate on the "Migrate the holders" Inbox item, which restarts the agent onto the current holder at its next idle moment in the same conversation, and that a shell tab is never migrated, so close it and open a new one.
- **Why no new item:** the upgrade path already raises it. Every holder from before this frame was spawned from another `humata2_holder` file, so its `holder_builds` sha differs, or it has none (D373), and the Holders item lists its agent session from the daemon's first look after start (`upgrade/holders.rs`; a box reports to the host, D378). Typing without the wait was a mixed-version fallback, which the force-upgrade rule bans.
- **Holders from before keyed input too (pick made, 2026-11-17):** the `KeyedInput::Unsupported` and `KeyState::Unsupported` fallbacks to plain typing are gone. `keyed_input` and `key_arrived` answer such a holder with the same `HolderError::Outdated`, so Type, review comments, the submit step and a start fresh summary are refused with "Migrate the holders" named and nothing typed (start fresh logs it as `SeedOutdated`). The refusal never stops the holder: it keeps running, with its program, until its session is migrated. `daemon/holder/tests/keyed_input.rs` `a_holder_from_before_keyed_input_is_refused_as_outdated` and `tests/return_after_text.rs` `a_holder_from_an_older_build_is_an_error_to_migrate_not_a_skipped_wait` show both: refused, then the same holder still attaches and its process is still running.
- **Tests:** `a_holder_from_an_older_build_is_an_error_to_migrate_not_a_skipped_wait` (`daemon/holder/tests/return_after_text.rs`) failed before the change: a stand-in old holder got `Unsupported`.
- **Upgrade:** no schema change. Old shells stay typable by hand but refuse `session type` with submit until reopened.
- **Reverse:** answer `ReadsKeys::Unsupported` again and type without the wait.

## D754: `humata session type` and MCP `session_type` press Enter by default; `--no-submit` and `submit: false` leave the text in the input, and the reply says `submitted` (Fable 5.2 ruling, 2026-11-17; amends D671 and B12). Decided (by Fable)
- **Pick:** the CLI appends one `\n` to the text unless it already ends in `\n` or `\r`; empty text presses Enter alone. `--no-submit` sends the text unchanged; there is no `--submit` flag. MCP `session_type` takes `submit?: boolean` (default true) with the same rule. The daemon still splits a trailing newline off as the submit (D671). `TypeResponse.submitted` (terminal.proto 4) is true only when the daemon wrote that return: false with no trailing newline and while queued. The CLI's JSON and MCP's result carry the daemon's `typed` and `submitted`; the CLI says "typed and submitted" or "typed, not submitted (press Enter or run again without --no-submit)".
- **Why:** B12 seen live: people and agents ran `session type` and the agent never got the text, because nothing pressed Enter.
- **Upgrade (D-UPGRADE):** `submitted` is an additive field; an old client ignores it, and an old daemon's reply reads as `submitted` false. No Migrate item.
- **Tests:** `cli/tests/sessions.rs` (`type_presses_enter_after_the_text_by_default`, `type_never_presses_enter_twice`, `type_with_no_submit_types_the_text_as_it_is`, `type_says_whether_the_daemon_submitted_it`); `daemon/humatad/tests/terminal.rs::typing_says_it_submitted_only_text_that_ends_in_a_newline`; `daemon/humatad/tests/mcp.rs::session_type_presses_enter_unless_told_not_to`.
- **Reverse:** drop the append and the flag; `submitted` can stay.

## D755: An agent never writes `shared/` on another machine of its channel; the refusal is its own, and its fix is the pull (Fable 5.2 ruling, 2026-11-17; keeps D33 pick 11, reads IN-17 and TP-4). Decided (by Fable)
- **Problem:** VERIFY's Copilot files flow 5 asked an agent on one machine to send a file to another machine's `shared/`. `write_place` in `daemon/access/src/policy.rs` refused it as `NotItsFolder` with the fix "work in the session's own clone or the channel's shared/ folder", which is wrong here: the target is the channel's `shared/`, just on the wrong machine.
- **Pick:** keep the hard refusal (D33 pick 11). IN-17's row is "Write into shared/ on its machine", so no rule covers another machine's `shared/`, and TP-4 is a pull: a session copies files in to its own clone or `shared/`. `daemon/access/src/verdict.rs` gets `Refusal::SharedOnOtherMachine { machine, here }` (kind `denied`, hard limit) with `Fix::PutInSharedHereForPull { machine }`. In `write_place`, `Root::Shared` on a machine that isn't the agent's own returns it; `Root::Clone` of another session and `Root::ChannelFolder` keep `NotItsFolder`, and another channel's `shared/` keeps the `other_channel` refusal. A copy whose landing is refused is refused before its source is judged, so the push fails at once. `daemon/humatad/src/status.rs` says: an agent writes shared/ only on the machine it runs on (its machine), so nothing was written to shared/ on the target machine; put the file in shared/ here, a session on the target machine can copy it in from there, or the user can send it with `humata cp`. Naming the machine is fine: it is one of the agent's channel's machines.
- **SPEC:** main updates FC-8 (an agent writes the `shared/` on its own machine; another machine's is pulled by a session there or sent by the user) and TP-4 ("`shared/` on its own machine").
- **Docs:** `api/MCP.md` `file_write` and `files_copy_in`, `api/CLI.md` §5.5 files put row; flows: `flows/cases/results/copilot-files.md` Flow 5 rerun in the pull direction, `flows/cases/agents/04-ask-ahead-and-grants.md` and `flows/cases/macro/12-files-across-sessions-and-machines.md` new cases.
- **Upgrade (D-UPGRADE):** no proto change; the error kind stays `denied` with source hard limit. Only the message and fix text change.
- **Tests:** in `daemon/access/tests/refusals.rs`, `an_agent_writes_shared_only_on_its_own_machine` (refusal and fix), `an_agent_pulls_another_machines_file_into_its_own_shared` (the pull needs only copy-in) and `a_copy_into_a_remote_shared_is_refused_before_its_source_is_judged`; `an_agent_writes_only_its_own_folders` keeps the clone and channel-folder cases. `daemon/humatad/tests/agent_send_machine.rs` still sees kind `denied` for the push and the pull landing.
- **Reverse:** add an IN-17 capability "Write into shared/ on another machine of its channel", Ask me by default, and have `write_place` return `need(...)` for it in place of the refusal.
- **Amendment (2026-11-17, VERIFY wave 3):** the message and fix name machines as people read them: the host is "This Mac", never its `ma_` id (`status::refused_named`, with the guard's `Names`).

## D756: An agent's bare last word of `cp|mv` that names no machine it can see is refused unless it has a `.` (Fable 5.2 ruling, 2026-11-17; amends D655, reads TP-1 and PL-12). Decided (by Fable)
- **Problem:** D655 read an agent's bare last word as a machine when one had that name, and otherwise as a path in its own clone. An agent sees only its own and its channel's machines (TP-1), so `humata cp … vm-b`, with vm-b outside the channel or misspelled, quietly copied into a file named `vm-b` in the agent's clone.
- **Pick:** for an agent principal only, on the last word of `humata cp|mv`, when the word has no `:` and no `/`: (1) a machine the agent can see is that machine; (2) otherwise a word with a `.` is a path in its own clone; (3) otherwise `CliError::NotAMachineHere { word }`, kind `invalid_field`, exit 2, and nothing starts. The fix names `humata host ls`, `./word` for a file and `word/` for a folder. `./name` and `name/` always mean the clone. The CLI strips a leading `./` and trailing `/` before it sends the path, because the daemon's path check (`RepoPath::new`) refuses `.` and empty parts. `name:` keeps the existing not-found. The person's reading is unchanged.
- **Why:** a typo or a machine outside the channel must not become a file in silence. The refusal is the same whether or not a hidden machine has that name, so it says nothing about machines the agent can't see.
- **Docs:** `api/CLI.md` §5.5; `flows/cases/macro/12-files-across-sessions-and-machines.md`; `flows/cases/results/copilot-files.md` Flow 5 attempt 1.
- **Tests:** `cli/tests/files.rs`, the six `an_agent…` cases after `a_send_to_a_machine_nobody_has_starts_nothing`.
- **Reverse:** fall back to the clone path.

## D757: Only a resume after a move starts a fresh agent when its conversation file is missing; any other restart saves the resume as failed. A resume check ignores the conversations of the store's other sessions (test triage, 2026-11-17; amends the move resume of D671's follow-ups, reads PER-13 and PER-14). Approved, with one required amendment
- **Problem:** the move resume (`start_moved`) started a fresh agent whenever a resumed agent's conversation file was missing. It was keyed on `Why::Moved`, but `restart_agent` also used `Why::Moved` for the own-copy and upgrade restarts. So a restart in the parent's own copy whose conversation was gone quietly started a new conversation, and `context_fork_test` saw it. Separately, a machine restart resumes all its agents at once, and one CLI's sessions can share a conversation folder. Another session's agent writing its own conversation looked like this agent starting another one (`another_grew`), so a fork's resume after a power cut failed with AgentStartedAnotherConversation (`power_cut_both_test`, RemoteFork).
- **Pick:** `Why::Restarted(line)` is the strict restart: a missing conversation is `ResumeFailed(ConversationFileMissing)`. `Why::Moved(line)` is used only by `resume_moved_agent`, which ResumeSession's AfterMove branch calls (`humatad/src/session_service.rs` `run_resume`). A move has carried and checked every file the agent wrote, so a missing file there is one never written, and the agent starts fresh. In `verify`, a grown sibling file that is another session's `launch.conversation_file` in the same folder, as the store lists it, isn't counted. An unknown file that grows still fails the resume.
- **Tests:** `daemon/revive/tests/kill_matrix.rs` `a_restart_whose_conversation_is_gone_fails_and_only_a_move_starts_it_fresh` and `another_session_writing_its_conversation_beside_it_does_not_fail_a_resume`; `daemon/humatad:context_fork_test`; `daemon/humatad:power_cut_both_test`.
- **Upgrade:** no schema or proto change.
- **Reverse:** make `restart_agent` use `Why::Moved` again, and count every sibling in `verify`.
- **Ruling:** Ruled by Fable, 2026-11-17. Approved: (a) is PER-15 as written (a lost conversation is never a silent fresh start), and the person's way out is intact, since "Couldn't resume" keeps Try again and Start fresh with a summary, which falls back to the saved scrollback when the transcript is gone. (b) hides no wrong-conversation resume: the agent is resumed by its pinned id (PER-12), so the real failure is the CLI starting a new id, which is an unknown file and still fails; an interrupted resume must also see its own file grow.
- **Required amendment (a):** `MODE_AFTER_MOVE` is a public `ResumeSessionRequest` mode, and `resume_session` trusts it (`session_service.rs`, `moved = request.mode == AfterMove`). Any caller allowed to resume a session can send it for a session that never moved and get the fresh start this decision reserves for moves. Honor the fresh start only when the move itself is the caller (the host's own `run_resume` path, or the moving machine's peer call) or the landed move's manifest shows no conversation was carried; otherwise treat AfterMove as the strict restart. Add a test that an AfterMove resume from an ordinary caller with the conversation gone is `ResumeFailed(ConversationFileMissing)`.
- **Amendment applied (2026-11-17):** `resume()` in `humatad/src/session_service.rs` honors the after-move fresh start only when the caller is the host (`Caller::User(UserVia::Host)`) and this daemon is not the host. Anyone else gets the strict resume. On the host, `resume_remote()` in `session_service/remote.rs` passes a caller's own AfterMove request on to the machine as a plain resume. The move itself doesn't go through there: on the host it calls `run_resume` with `moved` set, and for a box it calls `resume_session_on` from `move_session.rs`. Test: `daemon/humatad/tests/sessions.rs` `an_after_move_resume_of_a_session_that_never_moved_does_not_start_it_fresh`, red in run 80; `move_session_test` covers the real move still starting fresh.

## D758: A host's follow of a remote start, Desk start, resume or channel setup asks the machine again on the same link when it answers that it can't be reached, after a pause from 1 s doubling to 30 s, instead of waiting for a new link (test triage, 2026-11-17; reads D150 and D432). Decided
- **Problem:** `power_cut_machine_test` (`Start/Machine/Box@store.session_detail.put.committing` and `.committed`) left the host's start journal (`carry/remote_archive/start/<session>.json`) in place for good. Right after the box's restart, the box answered calls on the new link as unreachable (Unavailable with no `humata.v1.Error`, or MachineOffline) while that link stayed up. `follow_on_link` in `daemon/humatad/src/remote_work.rs` took that as "the link broke" and returned. `Links::reopen_on_new_link` starts a follow again only on a new link, so nothing asked again: the start was never answered and its journal never cleared. The lookup (`get_session`), the start asked again after a NotFound, a dropped watch and `settled`'s ask again all had the same return.
- **Pick:** on such an answer the follow pauses (1 s, doubling to 30 s) and asks again on the same link. When the link really goes, `reopen_on_new_link` drops the run as before. While the host waits on the machine's restart, `restart_wait` still bounds the wait. A refusal of the start asked again after a NotFound tells the start answered and ends the host's operation with the machine's words, the same as a refusal of the first ask again. A refused lookup ends the host's operation with the machine's words, where before it waited for good. `channel_attempt`, a channel's setup on one machine, had the same return on an unreachable answer to `create_channel` and on a dropped watch of its setup. It now pauses and asks again on the same link too; the same id gives back the one channel. A `create_channel` refusal with no `humata.v1.Error` and any other code now fails that machine's steps with the machine's words, where before it waited for a new link. A setup the machine forgets more than `MAX_ATTEMPTS` times still waits for the next link.
- **Tests:** `remote_work.rs` `a_start_the_box_answers_unreachable_while_its_link_stays_up_is_asked_again_until_answered` and `a_start_the_box_lost_is_asked_again_when_its_first_answers_on_the_new_link_are_unreachable`; `a_channel_the_box_answers_unreachable_while_its_link_stays_up_is_asked_again_until_made` and `a_channel_whose_setup_watch_breaks_while_its_link_stays_up_is_asked_again`; `daemon/humatad:power_cut_machine_test`.
- **Upgrade:** no schema or proto change.
- **Reverse:** return None from `follow_on_link` and `channel_attempt` on an unreachable answer again.

## D759: gpui's test scheduler is patched so a block with a timeout no longer deadlocks against work from another thread, and every live flow case has its own time limit, the PR review cases included (test triage, 2026-11-17). Decided
- **Problem:** `live_flows_test` shard 2 timed out on `pr_review::a_note_made_a_draft_and_the_persons_own_draft_post_in_one_review_per_pull_request`. The case's steps all passed. It hung in `drop(App)`: gpui's `App::shutdown` blocks with a timeout, and in gpui-pre-scheduler 0.3.12 `TestScheduler::block_until` then takes the rng lock and, while holding it, the state lock. Meanwhile a tokio thread (the terminal attach pump in `clients/live/src/link.rs`, passing on a notice) woke an app task, and `schedule_local` takes the state lock and, while holding it, the rng lock. Each thread waited on the other for good. Any live flow could hit it; this case's timing made it likely. The per-case limit (`desktop/tests/live_flows/leftovers.rs`) didn't end it because only `World` and `RemoteWorld` started a `CaseClock`. Four of the five PR review cases run on `pr_github::Fixture`, so they had no limit, and the whole target ran until Bazel's timeout.
- **Pick:**
  - (1) `apps/humata2/engine/patches/gpui-scheduler-lock-order.patch`, applied to gpui-pre-scheduler in every crate repository by a crate annotation in MODULE.bazel, reads the timeout range under the state lock and then takes the rng on its own. No path holds the rng while it takes the state any more.
  - (2) Each case's watch starts in `runtime()`, which every case calls first, and belongs to the case's thread (it ends with the case). A world or fixture adds its folders to it with `case_watches`, and `pr_review::started` adds the fixture daemon's folder. Past `CASE_LIMIT` the processes under those folders are ended. `CASE_GRACE` after that, a hang that holds no process (like this one) can't be ended inside the binary: the watch prints the stacks, names the case in the file `LIVE_FLOWS_HUNG` names, and exits. `tests/live_flows_shard.sh` then fails that case, runs the cases that hadn't reported again without it, and fails the shard naming it. One hang fails one case and the others still report.
- **Tests:** `engine:wakes_test` / `wakes_fast_test` `work_from_other_threads_while_the_app_blocks_does_not_hang_it`, which hung in run 88 without the patch and passed in run 89 with it (as did the six PR review rounds that hung in run 86); `live_flows_cases` `leftovers::tests::a_hung_case_has_the_processes_of_folders_added_after_its_watch_began_ended`; `desktop:live_flows_shard_test` (a stand-in binary whose case b hangs: b fails, a doesn't run again, c runs); `desktop:live_flows_test`.
- **Upgrade:** none; tests only.
- **Reverse:** drop the annotation and patch package. Start the watch in `CaseClock::start` again, and `exec` the binary once in the shard script.

## D760: Removing (or forgetting) a machine clears its skills sync state, so a machine added again under the name gets a first sync that asks (SK-5), and removing it takes it out of every channel. An add of a name whose removal hasn't finished is refused as being removed, not `name_taken` (VERIFY, 2026-11-17). Decided
- **Problem (1):** the host keeps each machine's skills sync state in `<state>/skills/<machine>.json` (`daemon/humatad/src/skills_service.rs`, `SkillsHost::record_path`), keyed by name only. RemoveMachine (`machine_service.rs` `remove_machine`) dropped the link, peer reports, stop and Inbox journal mark, but never this file. A re-add under the same name read the old manifest, so `plan_skills_sync` (`daemon/machines/src/skills.rs`, `first_sync: current.is_none()`) said `first_sync: false`, and `on_link_up` could sync it without asking.
- **Problem (2):** RemoveMachine answers with its operation before the removal runs, and the entry stays in `hosts.json` (as `Ready` until the job claims it, then `Removing`, kept `Removing` while the machine can't be reached). `Machines::plan_add` (`daemon/machines/src/manage.rs`) refused any listed name as `NameTaken`, which tells the user to pick another name.
- **Pick (1):** `SkillsHost::forget` (after any sync running to the machine ends, under its gate) deletes the record and drops its plan tokens. `Work::forget_journal`, which remove and Forget machine (LD-12) both call, now runs it. `forget_journal_now`, which an add afresh and a login enrollment call before the link starts, runs `forget_now`. That also clears a record left by a build before this fix. A record that can't be deleted is logged with a fix that says to delete it by hand.
- **Problem (3):** RemoveMachine left the machine's id in every channel's `machines`; only Forget machine (LD-12, `archive_lost`) took it out. The VERIFY remote lane saw removed ids still listed.
- **Pick (3):** the `Forgetting` hook gains `take_out_of_channels`, the same `leave_channels` Forget machine runs (`session_service/forget.rs`). It runs only once the removal has committed: in RemoveMachine's job, right after `Machines::remove` returns Ok (`Work::removed`, on the job's blocking thread), and for a removal that a cut-short run left `Removing` once `recover` finishes it at the next start, or a reconnect finishes it. A removal that fails (the machine can't be reached for its uninstall, or another add, reconnect or removal is running) leaves the machine listed and in every channel it was in, and a retry that succeeds then takes it out. A failure to update the channels comes after the removal is done, so it is logged with a fix (take it out of each channel by hand) and doesn't fail the removal.
- **Pick (2): refuse, don't wait.** `ManageError::BeingRemoved { machine }`: plan_add and plan_create return it when the entry is `Removing` or a removal was asked for (`Machines::removing`, set by RemoveMachine before it answers, cleared when `Machines::remove` ends). Its status is `ABORTED` with no kind (CLI exit 6, kind `aborted`). Its fix says to wait for the removal to finish and add again, and that a removal waiting on an unreachable machine finishes once the machine is back or Humata restarts. Waiting would hold the add's operation open for a removal that may wait on an offline machine indefinitely. Adds, reconnects and removals already refuse rather than queue (`Busy`, `NotReady`). The error has no source: no underlying error exists, only the state.
- **Tests:** `daemon/humatad/tests/skills.rs` `a_machine_removed_and_added_again_has_a_first_sync_that_asks_again`; `skills_service.rs` unit `a_forgotten_machine_starts_over_with_a_first_sync_and_no_picks`; `daemon/machines/tests/fake_ssh.rs` `an_add_while_a_removal_is_on_its_way_is_refused_as_being_removed_until_it_finishes` and `an_add_while_a_removal_waits_for_its_machine_is_refused_as_being_removed`; `daemon/humatad/tests/machines.rs` `an_add_while_the_machine_is_being_removed_is_refused_until_the_removal_ends`; `daemon/humatad/tests/forget_machine.rs` `removing_a_machine_takes_it_out_of_every_channel` and `a_removal_that_fails_leaves_the_machine_listed_and_in_its_channels`.
- **Docs:** `api/CLI.md` host add and host remove rows; `api/GRPC.md` §10.
- **Upgrade:** no schema or proto change. A stale record from an earlier build is deleted at the next add afresh.
- **Reverse:** drop the skills forget from `forget_journal`/`forget_journal_now`, the `Work::removed` calls (RemoveMachine's job, `recover_in_background`, `reconnect`), and have `name_free` return `NameTaken` again.

## D761: Copilot's ask-user form is an Inbox question: its `preToolUse` hook, matched to `ask_user`, holds while the question waits, the session needs you meanwhile, and the reply reaches Copilot as the reason the form was skipped (VERIFY copilot-t1, 2026-11-17). Decided
- **Problem:** Copilot 1.0.113 asks the person through its `ask_user` tool: a form in its TUI (a message plus an elicitation `requestedSchema`, whose fields may offer an `enum` of choices). It raises no permission ask, so no Inbox request came, `humata session status` kept saying `working`, and only someone looking at the terminal found out (flows/cases/results/copilot-t1.md, flow 3; the lane answered with `humata session type`). Copilot's own events.jsonl shows `tool.execution_start` of `ask_user`, then `human_response.recorded`, then a `postToolUse` hook: tool hooks fire for it.
- **Signal:** the hooks plugin Humata already writes for Copilot (D193) gets a `preToolUse` hook with `"matcher": "ask_user"` running `humata hook copilot question` (held up to `HOLD_SECONDS`, like the permission hook). The matcher keeps every other tool call from waiting on a CLI run; the daemon reads `toolName` all the same. Not taken: tailing events.jsonl (Humata reads it for the conversation mirror, not live, and a watcher can only type an answer into the form, which a choice list doesn't take); a permission ask (Copilot raises none).
- **Pick:** `SessionService.Hook` takes the event `question`. `agent_hooks::parse_question` reads the payload (`toolArgs` an object or a JSON string; `message` or `question`; each schema field's title and description on a line of its own; choices from `enum`, `oneOf`/`anyOf` consts, array `items`, or a `choices` list). The daemon raises the existing question card (`REQUEST_KIND_QUESTION`, `QuestionPreview` with the whole question and every choice), held like a permission card (D191): the same question again holds the same card, the last hook letting go withdraws it (CALL_ENDED), Answer all skips it. It takes `humata inbox reply <id> <text>` (one of the choices or the person's own words) or Deny; Approve is refused with a fix that says to reply. While the hook holds it the session is NEEDS_INPUT (the D158.7 `Asking` hold), until it is answered, denied or withdrawn. The answer goes back as `{"permissionDecision":"deny","permissionDecisionReason":"The user answered this question in Humata's Inbox ... Their answer: <text>"}`: Copilot's docs say that reason is shown to the agent, and the form never opens. A Deny says the person declined, with their note. No answer (the 3590 s cap, a withdrawal, or Settings › Agents › Ask for permission in the Inbox off, D199) is empty output, and Copilot's own form opens.
- **Owed live check:** that Copilot 1.0.113 runs a `preToolUse` hook with a `matcher` for `ask_user`, and that the model goes on with the answer in the deny reason rather than asking again. Until then the form in the TUI still works whenever the hook gives no decision.
- **Tests:** `daemon/agent_hooks:questions_test` (the captured form and a choices form become one question with every choice; another tool, a permission payload and another agent raise none; an answer, a decline and no answer render as Copilot reads them; the plugin holds `ask_user` for an hour); `daemon/humatad:hooks_test` `copilots_ask_user_form_is_an_inbox_question_and_the_reply_reaches_it`, `declining_copilots_question_tells_it_and_ends_the_wait`, `a_question_whose_hook_lets_go_is_withdrawn_and_another_tool_raises_none`; `cli:hook_test` `an_ask_user_form_goes_to_the_daemon_as_a_question`.
- **Docs:** `api/CLI.md` hook row, `api/GRPC.md` SessionService row, `HookRequest.event` comment in `sessions.proto`.
- **Upgrade:** no schema or proto change (a comment only). The daemon rewrites the Copilot plugin at its start; a session started before keeps the old hooks until it restarts. An older CLI refuses `question` as usage, which `|| true` turns into no decision, so the form opens.
- **Reverse:** drop the `preToolUse` entry from `copilot_plugin` and the `question` arm of `hook_of`.

## D762: Every write into an agent tab passes one input gate that refuses text while the agent asks a question on its screen (folder trust) or when its screen can't be read, so a lost submit is never reported and Humata never answers the trust question for you (VERIFY copilot-remote rerun finding 1 and copilot-files rerun, 2026-11-17; SPEC CV-6). Decided (by the fixing agent; reversible)
- **Problem:** after a move to a remote machine (copilot-remote rerun, Flows 3 and 4) and on a fresh remote session (copilot-files rerun, f5), `humata session type <s> "text"` answered `typed: true, submitted: true` but the text never reached Copilot; the screen then showed only "MCP Servers reloaded", and a second type worked. Root cause: Copilot 1.0.113 shows "Confirm folder trust" in every folder not remembered as trusted (CV-6 lets it ask after a move, since a session-only "Yes" isn't carried). The dialog reads keys in raw mode, so BUG-T1's `ready_for_text` (`await_keys`) said ready. The text went into the dialog, `shown` timed out after 5 s (its result is ignored by design, for pastes shown as summaries), and the separate Enter answered the dialog with its highlighted "1. Yes". Copilot then loaded its MCP servers ("MCP Servers reloaded"). So Humata both lost the text and granted trust the user never gave (against CV-6).
- **Pick:** one input gate, `input_gate::guard(socket)` (`daemon/humatad/src/input_gate.rs`), async and keyed by the holder socket, is asked before any text is written into an agent tab. It rebuilds the screen the holder shows now from the output it kept (`holders::screen_at`: a read-only throwaway `vt_authority` screen, `Authority::screen_text`, the active area only) and looks for a known question (`screen_question::asked_on`: "Do you trust the files in this folder?", box drawing and wrapping ignored). A question there is `InputRefused::Question`; a screen it can't read within 3 s (no holder, attach failed, engine error, timeout) is `InputRefused::Unreadable`: it fails closed and nothing is typed blind. Type answers FailedPrecondition for a question (answer it in the Inbox (D767) or on screen; Humata won't answer it; Enter alone accepts it for this session only) and Unavailable for an unreadable screen (type again in a moment), types nothing and releases its key. Enter alone is still sent: it is the person answering on purpose. Shell tabs are not gated.
- **Not taken:** holding Type's text until the agent is ready (the question waits for a person, for as long as they take; a held text would then submit into whatever came next; the background notes and seeds do wait); typing when the screen can't be read (fail open, the first version of this pick: an unreadable screen says nothing about where the text would go); refusing every text the program didn't show (Claude shows a long paste as a summary, which would no longer submit); answering the trust question automatically (CV-6); patterns for agents with no recorded screen.
- **Scope:** every write of text into an agent tab goes through the gate. Type (`type_in`: the CLI, MCP `session_type`, the composer, machine scripts through `type_for_script`; a remote session's Type is relayed and gated by that machine's daemon); the review comments' paste (`type_comments`, keyed and plain; the relayed one is a remote Type; the return pressed after the paste is only a return and isn't gated); a rebuilt session's note (`rebuild.rs` `type_note`, which now waits in the background up to 10 min for the gate); and the start-fresh and fork-handover seed (`start_fresh.rs` `type_seed`, which waits up to 10 min, then fails as `SeedNotReady` and is kept for the next restart). The local queued send (`SendQueuedNow`) is not built and types nothing. A start's first message (`sessions/src/start.rs`) and revive's interrupted note (`revive/src/agent.rs`) are in crates without humatad: D763 gives them a `WriteGate` trait that humatad implements with `input_gate::guard`. Not covered: a question drawn after the gate read the screen but before the text landed (a window of milliseconds).
- **Tests:** `daemon/humatad:terminal_test` `text_typed_while_the_agent_asks_folder_trust_is_refused_and_never_answers_it` (a fake agent asks trust as a select list in raw mode; text with and without submit is refused and the question still waits; Enter alone answers it; then the text submits); `daemon/humatad:humatad2_unit_test` `input_gate::tests` (a holder that can't be reached, and a socket that never sends a screen, both refuse as Unreadable, which Type answers as Unavailable) and `screen_question::tests`; `daemon/vt_authority:authority_test` `the_screen_text_is_what_the_active_area_shows_now_without_history`.
- **Upgrade:** daemon only; no proto, schema or holder change. A remote machine's daemon needs this build for its sessions to get the check.
- **Reverse:** drop the `input_gate::guard` calls (terminal.rs `type_in` and `type_comments`, rebuild.rs `type_note`, start_fresh.rs `type_seed`) and the `WriteGate` impl.
- **Follow-up:** add the first-run questions of Codex and Claude Code to `screen_question::ASKED` once their screens are recorded live (VERIFY with those agents); none is in the tree or the VERIFY evidence now, so none is guessed.

## D763: A start's first message is typed only once the agent's input is clear (D762's gate) and its return is pressed only once the agent shows it. Until then the session needs you; one still unsent after 30 min is an Inbox request with its whole text (VERIFY copilot-t1 BUG-T1b, 2026-11-17; amends D671, uses D762 and D765). Provisional
- **Cause:** `type_then_return` (`daemon/holder/src/client.rs`) waited up to 15 s for the text to show, ignored the result, and pressed return anyway. Copilot 1.0.113 in a folder it doesn't trust asks "Confirm folder trust" first, reading keys raw, and holds what is typed unshown. The blind return answered that question for the person (against CV-6), and the message was left typed in Copilot's input. A VM pty probe showed Copilot keeps the text: once the dialog closed, its input read `❯ reply with the single word pong`. This Mac passed only because the verify lanes pre-trust their root.
- **Holder:** `type_then_return` returns `Typed::Submitted` or `Typed::Held(HeldReturn)`; a program that hasn't shown the text within 15 s gets no return. `Shown` (`shown.rs`) counts the `[Paste` summary Copilot and Claude Code show for multi-line text as shown. `deliver(socket, text, pause, held, ready, limit)` sees a held or untyped text through: with a `ready` check it types only when the input is clear, presses return once the text shows, and types the text again once when the input is clear but the text still isn't shown 15 s later (the program dropped it); without one it never retypes, as a dropped text can't be told from a held one.
- **Input ready:** the agent reads keys (`await_keys`, raw mode) and D762's `input_gate::guard` says Ok (no known question on its screen, and the screen could be read). The sessions crate has no screen, so it asks through `humata2_sessions::WriteGate`, keyed by holder socket; humatad's `input_gate::StartGate` maps `InputRefused` onto `GateRefusal`.
- **Start:** `FirstMessage::send` (`daemon/sessions/src/first_message.rs`) asks the gate once; clear and shown within 15 s, it submits before the start is Done as before. Otherwise the start finishes and a background follow runs `deliver` for up to `ANSWER_WAIT` (30 min). While it waits, `HeldMessages::waiting` holds `Asking` (status `needs_input`, not `working` or `probably_idle`); a submitted message lets go, and the turn's hooks take over. Still unsent, `HeldMessages::unsent` (`daemon/humatad/src/start_message.rs`) raises a question of the session's own with the whole text and the choices "Send it now" and "Leave it unsent", holding `Asking` until it is answered. "Send it now" tries again through the same path. It goes through `raise_question`, not a host ask, because a host ask stays in the daemon's memory: a remote machine's never reached the host. A session question on a machine is saved and journaled (D744), so the host's Inbox shows it and its answer comes back down the link.
- **Revive:** the resumed agent's "interrupted" note (`daemon/revive/src/agent.rs` `verify`) asks the same gate, now `humata2_revive::WriteGate` (sessions re-exports it), set in `ReviveConfig.gate`. While the gate says Question, nothing is typed and the resume's deadline moves out with each look (every 500 ms), so a person answering in their own time doesn't fail the resume. Unreadable fails closed: nothing is typed, and the deadline isn't moved. Once clear, `deliver` types the note and presses return once it shows, retyping a dropped note once.
- **Not changed:** the other typing paths are D762's. A screen that stays unreadable for the whole resume timeout still fails as `AgentStartedAnotherConversation`; no new `ResumeFailure` variant.
- **Tests:** `daemon/holder:return_after_text_test` `a_return_waits_while_the_program_asks_a_question_of_its_own`, `a_text_the_program_dropped_while_it_asked_arrives_once`, `a_text_the_program_kept_while_it_asked_is_not_typed_twice`, `a_text_whose_input_never_clears_is_not_sent`, `a_paste_shown_as_a_summary_is_submitted`; `daemon/holder:holder_unit_test` (`shown.rs` summary cases); `daemon/humatad:humatad2_unit_test` `start_message::tests::a_waiting_start_message_needs_you_and_an_unsent_one_is_asked_about_with_its_text`; `start_message::tests::a_remote_machines_unsent_start_message_is_asked_about_in_the_hosts_inbox_with_its_text`; `daemon/revive:kill_matrix_test` `a_resumed_agent_asking_a_question_is_told_once_it_is_answered_however_long_that_takes`.
- **Upgrade:** daemon and holder library only; no proto or schema change. A remote machine's daemon needs this build for its starts.
- **Reverse:** have `type_then_return` press return after `SHOWN_LIMIT` again, set `Starter.first_message` to the default (no gate, no Inbox) and `ReviveConfig.gate` to `None`.
- **Amendment (VERIFY rerun2, 2026-11-17): a first message left in the input gets one more Enter.** Live, once the trust box was answered, the held start message was typed and shown and its return pressed, but in 1 of 2 sessions Copilot left it in its input. Now, 3 s (`SUBMIT_WAIT`) after a first message's return (submitted at once, or after a held wait), `FirstMessage` asks the gate whether the agent's input still shows the message (`WriteGate::unsent`, default `false` so revive's gates are unchanged; humatad's `StartGate` reads the screen: the last row starting with `❯` or `>`, with the rows below it, still holds the message's last line). If so, and the gate still says the input is clear (no question on the screen, screen readable), return is pressed once more, never twice; otherwise nothing more is pressed. The start doesn't wait for this look. Tests: `daemon/sessions:sessions_unit_test` `first_message::tests`; `daemon/humatad:humatad2_unit_test` `input_gate::tests` (`input_shows`). Reverse: drop the `confirm_logged` calls in `first_message.rs`.
- **Amendment (VERIFY wave 3, 2026-11-17): a held first message is typed again only if the input stayed clear without it.** Live on a remote machine, 4 sessions started with `--message` while Copilot's trust box was up; once answered, 2 of the 4 submitted the message twice in one submission ("…then stop.Create a file…then stop."). Cause: `deliver` typed the text again when one gate look said the input was clear and the text didn't show within `SHOWN_LIMIT`. A look taken while Copilot redraws its box (it draws it in steps, and an attach resizes it) can find no question for a moment; the 15 s wait then ran while the box was still up, and the retype landed behind it next to the text Copilot kept. Now the input must stay clear, checked every `READY_EVERY`, for the whole `SHOWN_LIMIT` without the text showing before it is taken for dropped; a look that finds the question again goes back to watching. There is still one typing path (the start's `deliver`); the trust reply types only the choice. Test: `daemon/holder:return_after_text_test` `a_text_kept_behind_a_question_that_looked_clear_for_a_moment_is_submitted_once` (red in run 116 with exactly the live doubling). Reverse: put back the single `once_shown(SHOWN_LIMIT)` before the retype. Second path, same wave: the 30 min card's "Send it now", pressed while the box was still up, then the box answered in the terminal, also doubled it: the resend typed as soon as no question was up, next to the text Copilot kept. Now a resend (`first_message.rs` `kept_once_clear`) waits for the question to go, lets the agent settle (`SUBMIT_WAIT`), and when the input already shows the text only presses return (then the usual confirm); otherwise it types. Tests: `daemon/sessions:sessions_unit_test` `a_resent_message_the_agent_kept_behind_its_question_gets_a_return_not_a_second_typing`, `a_resent_message_the_input_doesnt_show_is_typed`, `a_resent_message_behind_a_question_still_up_presses_nothing` (written with the fix, not shown red first, for the 22:30 freeze).

## D764: A Repos and machines grid cell shows its repo's main clone as the host records it, or MACHINE_OFFLINE when the host's last look at that machine wasn't ONLINE; it counts live sessions only (CH-9, 2026-11-17). Decided
- **Problem:** `GetGrid` (CH-9, TP-1, `humata channel grid`) answered "not built". `GridCell.state` offers READY, CLONING, FAILED and MACHINE_OFFLINE, but the host keeps one main-clone state per channel repo (`ChannelRepo.main`), not one per machine. CH-9 says the grid shows no clone health and marks offline machines.
- **Pick:** one cell per channel repo × channel machine, repos then machines in the channel's order. `sessions` counts the channel's sessions placed on that machine (an empty machine is the host) whose detail lists that repo. Archived sessions and deleted Desk sessions count nothing; sessions on machines or repos the channel no longer lists show nowhere. `state` is MACHINE_OFFLINE for a machine other than the host whose last seen reachability (the one `host list` shows) isn't ONLINE, or that the machine list doesn't have; otherwise the repo's main state (Cloning, Ready, Failed). The read is CHANNEL_READ on the channel, asked like GetChannel. It is not relayed (host only, as `api/CLI.md` says). No probe runs: the grid reads what the host already knows.
- **Not taken:** a per-machine main state (no store field holds it; the machine's own clone health is not the grid's job per CH-9); asking each machine live (slow, and an offline one would hold the reply).
- **Tests:** `daemon/humatad:humatad2_unit_test` (`channel_service/grid.rs`: counts per cell, main state vs offline, sessions on dropped machines or repos, no machine list); `daemon/humatad:channel_grid_test`.
- **Docs:** `api/GRPC.md` ChannelService row; `flows/cases/README.md` gap 49.
- **Upgrade:** none: no schema or proto change.
- **Reverse:** put back the `not_built` arm in `get_grid` and drop `channel_service/grid.rs`.

## D767: A question an agent asks on its own screen, such as Copilot's "Confirm folder trust", raises an Inbox question with its choices; the session needs you while it shows, and only the person's reply picks a choice (VERIFY copilot-remote, 2026-11-17). Decided. Superseded by D845 (INBOXHOOKS)
- **Problem:** Copilot 1.0.113 asks "Do you trust the files in this folder?" for every fresh remote session and after every move, before its input box shows. No hook fires for it, so no Inbox request came and the status said working. D762 only stops Type from typing into it.
- **Signal:** the session's agent watch (`session_service/watch.rs`, SE-21) already times the agent's output. Each time the output goes quiet (`QUIET_AFTER`, 3 s), `screen_ask::quiet` reads the tab's screen from its holder (`holders::screen_at`) and asks D762's `screen_question::asked_on`. That is one matcher, and only agent tabs are read. Not taken: a timer of its own (the quiet spell is when a question sits on the screen), hooks (none fire).
- **Pick:** the card is the existing QUESTION card held as D761's are (`Inbox::raise_question`): the question line, the folder shown just above it, every numbered choice the screen lists (`1. Yes`, `2. Yes, and remember this folder for future sessions`, `3. No (Esc)`), and a note that Yes lets the agent use the folder's files and Humata never answers for you. The card takes only one of its choices (its words, ignoring case, or its number), kept as the choice's words; anything else is refused with the list. Approve is refused (D761). While it is held, `Asking` keeps the session NEEDS_INPUT. A reply re-reads the screen and, only if it still asks the same question with the same choices, moves the highlight there with arrow keys and presses Enter. Deny types nothing, and the screen keeps the question for the person; the same screen isn't asked again. The screen no longer asking (answered in the terminal), the agent exiting or a new holder withdraws the card (CALL_ENDED). D199's Ask for permission in the Inbox off keeps it on the screen only. Humata never picks a choice by itself.
- **Owed live check:** that Copilot's trust list moves with arrow keys and takes Enter as the tests' agent does, on a fresh remote session and after a move.
- **Tests:** `daemon/humatad:screen_questions_test` (`folder_trust_on_the_agents_screen_is_an_inbox_question_answered_only_by_the_person`, `folder_trust_answered_in_the_terminal_withdraws_the_question`, `declining_folder_trust_in_the_inbox_types_nothing`); `daemon/humatad:humatad2_unit_test` `screen_ask` tests (the question, folder, choices and highlight read off Copilot's screen; arrow moves).
- **Docs:** `api/CLI.md` `humata inbox reply` row.
- **Upgrade:** none: the daemon looks at screens from its start, and a session whose screen asks now raises the card at its next quiet spell.
- **Reverse:** drop the `screen_ask::quiet` call in `watch.rs`.
- **Amendment (VERIFY rerun2, 2026-11-17): the card offers every choice the box shows, and a reply reaches the box.** Live with Copilot 1.0.113, cards offered only `["Yes"]` or `["Yes", "Yes, and remember…"]` while the screen showed all three rows, and `humata inbox reply <rq> Yes` marked the card answered but the box stayed up (only a bare Enter typed in the session cleared it). Cause: the card was raised from the first quiet spell's read of the screen, which listed fewer rows than the finished box (inferred from the cards against the captured screens: the raw output of that moment wasn't kept), and a later look at the same question was skipped because `look` compared only holder and question, so the card never caught up. On the reply, `pick` re-read the screen, found three choices where the card had one or two, and returned without typing or saying anything. The folder never reached the card either, as Copilot puts four rows of its own between the folder and the question and only four rows above were searched. Now: (1) `Open` keeps what the card shows; a look that reads different words or choices withdraws the old card and raises one with the new list (its key carries the choices, so the two never merge); a moved highlight alone isn't a new card. (2) A reply re-reads the screen and, while it still asks the same question and still lists the chosen choice's words, moves from the highlight there now to that choice's row there now (`keys_for`), then presses Enter; a screen that stopped asking or dropped the choice types nothing and says why in the log (`NotPicked`). (3) The folder is the nearest path row anywhere above the question. (4) A choice's key hint is left off its words (`No (Esc)` is `No`), so replying `No` (or `3`) presses that row; Deny still types nothing. Remote sessions: the card is raised, held and picked by the machine's own daemon, whose holder it types into; the host's answer comes down the link (D744), so the same code path serves both. D762's gate doesn't apply to these keys (they go through the link, not Type). Tests: `daemon/humatad:screen_questions_test` `a_trust_box_drawn_in_steps_offers_every_choice_and_no_picks_the_no_row`; `daemon/humatad:humatad2_unit_test` `screen_ask::tests` (`copilots_captured_trust_boxes_give_every_choice_and_the_highlight`, from both captured screens with made-up folders; `a_reply_moves_the_highlight_to_its_row_on_the_screen_now`; `a_reply_to_a_card_read_off_a_box_still_drawing_picks_its_row_on_the_full_box`; `a_reply_picks_nothing_once_the_screen_stops_asking_or_drops_the_choice`; `a_key_hint_is_left_off_a_choice_and_other_brackets_are_kept`). Still owed live: the pick on Copilot itself. The held start message that once needed a second Enter is D763's amendment.
- **Amendment (VERIFY wave 3, 2026-11-17): the card follows the screen while the agent keeps writing, and a reply is checked against the screen.** Live, cards raised off a box drawn in steps kept two choices for over a minute while the screen showed three, until an attach read the screen again; and a reply to a box already answered on the screen came back answered though nothing was pressed. Cause: the screen was read only when output went quiet, and Copilot's animated logo keeps writing. Now, while a session's card is up, its output also has the screen read again at most every `RELOOK_EVERY` (300 ms, a throttle, not a wait for quiet), so the card is raised again with every choice, or withdrawn once the screen no longer asks (`screen_ask::Asks::withdraw_gone`, the one path for that). That same read drops the card's hold on the session's Needs you (D775 amendment). Screen cards are raised with `Inbox::raise_screen_question`, which keeps a check (`StillAsked`) on the entry: `answer_as` runs it on a reply, before it is settled, and a screen that no longer asks that question or lists that choice withdraws the card, and the reply gets `request_withdrawn`. A screen that can't be read within 3 s refuses the reply as Unavailable (`step_failed`, reply again) and leaves the card pending. On a machine the check runs where the session runs, as relayed answers go through the same `answer_as`. Tests: `daemon/humatad:screen_questions_test` `a_trust_box_that_finishes_drawing_while_its_agent_keeps_writing_ends_with_every_choice` and `a_reply_after_the_box_was_answered_on_the_screen_is_withdrawn`. Reverse: drop the `screen_ask::output` call in `watch.rs` and raise screen cards with `raise_question` again.

## D768: An agent on a machine's own daemon looks machines up, lists and stats other machines' files and copies between machines through the host, which answers as that agent; `this-mac` there names the host Mac (VERIFY copilot-files BUG, 2026-11-17; uses TP-8, D672, D755). Decided
- **Problem:** on vm-a, `humata cp this-mac:shared/agent-note.txt shared/` and `humata host ls` failed "MachineService.ListMachines isn't built into this humatad2 yet" (exit 7) and told the agent to update Humata, which was current. The machine-role daemon (`machine_service/box_storage.rs`) answered ListMachines and GetMachine `not_built`, and judged an agent's read of the host's files locally: the channel rule's Ask became `status::needs_approval`, Unimplemented with no kind, which agents read as `peer_version_unsupported`. The machine has no machine list and no links in any case; only the host does.
- **Pick:** five more Uplink rows (`away/registry.rs`): MachineService ListMachines and GetMachine, FileService ListDirectory and Stat, TransferService StartTransfer. A machine's daemon sends an agent's call of these up as a `ForwardedCall` (`away/up.rs` `ask_host`) when it isn't about this machine alone: the lookups always, a list or stat that names another machine, and a copy whose source or destination names another machine (this machine's own side goes up with no machine named, and the host fills in the link's id: the host knows the machine by that id, not by the `ma_…` id the machine's daemon gives itself, which the 19:05 live rerun showed failing as "session … is on machine vm-a, not ma_…"). The host's `BoardSvc::run_forwarded` runs it with its own services (`HostCalls`) as that session's agent, under the host's rules; the machine adds nothing. The host's answer to a machine agent's ListMachines puts This Mac first (`is_this_mac`) when the channel has it, as the machine can't name the host otherwise. A user's or script's ListMachines on a machine's daemon stays `not_built` (the app talks to the host). ReadFile of another machine from a machine is refused with a fix (copy it in first): a stream isn't forwarded. With the host away, the call fails Unavailable `host_unreachable` with "wait until the host is linked".
- **CLI:** `this-mac` for an agent on a machine-role daemon resolves to the listed `is_this_mac` machine (not the daemon's own machine, as D672 does elsewhere); `host ls` doesn't add This Mac twice.
- **Errors:** `status::needs_approval` is PermissionDenied with kind `denied` and the capability, not Unimplemented. `not_built`'s fix no longer says to update: it says to do it from the app and that updating helps only once a release builds it. The CLI's bare-Unimplemented text says the CLI and humatad come from different releases and to migrate from the host's Inbox.
- **Received:** a send to another machine by name alone lands in its `received/`, which an agent can't write; the refusal is D755's pull hint (`SharedOnOtherMachine`, message now "writes only on the machine it runs on"), not "leads outside Humata's folders".
- **Asked reads:** FileService reads still used `guard.require`, so a read the channel's rule Asks about was refused at once and raised no Inbox card, on the host and on a machine alike. Settled by the ruling and its amendment below.
- **Not taken:** giving machines their own machine list or links (two sources of truth; D227 keeps the host the one judge); forwarding ReadFile's stream.
- **Owed:** MCP's file tools pass a machine name as typed; "This Mac" isn't a name the host resolves, so an agent's MCP copy should use `this-mac` or the id.
- **Tests:** `daemon/humatad:humatad2_unit_test` (`away::registry` rows); `daemon/humatad:away_forward_test` (a machine agent's ListMachines, Stat and StartTransfer go up as forwarded calls); `cli:this_machine_test` (`this-mac` on a machine picks the host; `host ls` lists it once); `daemon/humatad:agent_send_machine_test` (an agent on box-1 lists machines and pulls from This Mac on the rig); `daemon/access:refusals_test` (received/ refusal).
- **Upgrade:** both ends: a machine at an older release still answers `not_built`, and a host at an older release refuses the new rows as unknown methods. D-UPGRADE's Migrate covers it; no schema change.
- **Reverse:** drop the five rows and their arms in `run_forwarded`, and the `ask_host` calls in `box_storage.rs`, `files/forward.rs` and `transfer_service.rs`.
- **Ruling (Fable, 2026-11-17):** an asked file read must not be a dead end. It raises an Inbox approval carrying the agent, session, machine, path and operation, and the call answers at once with `approval_pending` and the request id (the same gate other asked calls use); it does not hold the read open. The agent waits on the request or asks again with the same key. One card per session, capability and path, so retries don't stack. A machine agent's forwarded read is asked on the host (D227). `status::needs_approval` goes away for file reads.
- **Amendment (ruling built, 2026-11-17):** FileService's reads (ListDirectory, Stat, ReadFile, FindFiles, and the host's relayed reads of another machine in `files/forward.rs`) go through `files::Reads::require`: on Ask it calls `Inbox::ask_read` (`inbox/deferred.rs`), which raises the ordinary deferred card (same kind, preview and requester as other asked calls) and answers `approval_pending` at once. The card is keyed by session, capability and the place's machine, root and path (not the entry kind, so a stat and a read of one file share it); a repeat of the read meets that card: still pending, the same `approval_pending`; approved, the read goes through, for as long as an approved call's result is kept (24 hours, in memory); denied, PermissionDenied `denied` with the request id, after which the next read asks afresh, as it does after a withdrawal or a restart. Approving runs nothing: the agent repeats the read, since ReadFile's stream can't be replayed. The approval's kept result type is `humata2.ReadApproved` with no bytes (`status::READ_APPROVED`), so MCP's `inbox_wait` answers `repeat_call: true` with a note, not a bare approval, and `humata --wait` repeats the call and prints its result; the read's `approval_pending` fix (`status::read_approval_pending`) says to repeat the read once approved, where other calls' says not to repeat. A machine agent's read of This Mac goes up as a `ForwardedCall` and is asked on the host's Inbox by the same code. `needs_approval` stays only for a non-agent caller the rules ask about, which the rules don't do today. Tests: `daemon/humatad:inbox_test` (one card, reuse, approve then read, deny, a stat and a stream read sharing a card), `daemon/humatad:agent_send_machine_test` (`a_machine_agents_asked_read_of_this_mac_waits_on_the_hosts_inbox`). Reverse: put `guard.require` back in those reads and drop `ask_read`.
- **Amendment (approved reads run, 2026-11-17):** approving an asked ListDirectory, Stat or ReadFile now runs that read, on this machine or relayed to another, and `WaitForAnswer` returns its result (`ListDirectoryResponse`, `FileEntry`, or one uncompressed `FileChunk` of up to 8 MiB, `last` false when the file goes on), so `humata --wait`, `humata files cat` and MCP's `inbox_wait` show the real result; the pending fix says the wait returns it. FindFiles and Desk versions (now on `ask_read` too, `desk_service/versions.rs`) still keep `humata2.ReadApproved` and the agent repeats them; so does a relayed read whose machine was offline when the card was raised. **Reversal:** the card key now includes the operation (list, stat, read, find, versions), so a stat and a read of one file get a card each, reversing "a stat and a read share one card" above: each card's wait must return that read's own result. Held read bytes are memory, so they go to the first taker: the first wait that delivers them, or a repeat of the read that goes through, drops them and the card's result turns into `humata2.ReadApproved`; bytes nobody takes are dropped after 10 minutes (`READ_HELD`). The approval itself keeps the usual 24-hour keep time, so a repeat still goes through after that. The transfer-end reads stay on `guard.require`: they follow `user_only`, so an agent never reaches them. Tests: `daemon/humatad:inbox_test`, `humatad2_unit_test` (`inbox::deferred` paused-clock expiry, `mcp::approved_read_tests`), `cli:waiting_test`, `cli:files_test`. Reverse: pass `None` as the run in `Reads::require` and drop `once`/`taken`.

## D769: `humata cp a b`, with one source and `b` a new name or an existing file, copies to the file `b`, as plain cp does; StartTransfer takes the landing name (VERIFY copilot-files BUG, 2026-11-17; amends D756). Decided
- **Problem:** a host agent's `humata cp note.txt vm-b.txt` answered "there is no vm-b.txt there": D756 reads the dotted word as a path in the agent's clone, and StartTransfer's destination must be an existing folder; each source lands under its own name in it.
- **Pick:** `StartTransferRequest.name` (field 7): the one source's name in the destination folder; empty keeps its own. It is refused with more than one source and when it isn't one plain path part. The engine's `start_as` lands the one top under it (on-existing rules apply to that name). The CLI, for one source and a destination without a trailing `/`, Stats the destination: a folder takes the source under its own name; not found, or a file, sends the parent folder with `name` set to the last part. Any other answer (a refusal, unreadable) is sent as a folder, as before, and the daemon says what is wrong.
- **Not taken:** guessing from a dot in the name (folders have dots too); a daemon-side Stat in StartTransfer (the destination's meaning is the CLI's, like cp's).
- **Tests:** `daemon/transfers:transfers_test` (a named top lands under the new name; two sources with a name are refused); `cli:files_test` (`cp note.txt vm-b.txt` sends the clone root and `vm-b.txt`; an existing folder keeps the source's name).
- **Upgrade:** additive proto field; an older daemon ignores `name` and fails as before. Ships with D768.
- **Reverse:** stop setting `name` in `commands/files.rs`; the field stays unused.

## D770: every Settings section is served; the portable ones live in `config/settings.toml` (VERIFY worker, 2026-11-17). Provisional
- **Pick:** UpdateSettings serves default_agent, terminal, retention_defaults, long_lines, skills and notifications, and UpdateChannelSettings serves default_agent (with clear_default_agent) and retention. None of them returns `not_built` any more. The global sections other than skills are kept in the humata home's `config/settings.toml` (portable config, written whole). Skills go in the state folder's `skills-settings.json`, the file skills sync already reads, so SK-3/SK-6 take the change at once. A channel's own default agent and retention go in the store keys `channel.<id>.default_agent` and `channel.<id>.retention` at the channel's revision. The Settings revision is the newest of every section's saves; the config file's saves are counted under the store key `settings.sections`. GetSettings fills every section: Wrap (ST-7), retention all off (ST-14, LD-16), skills auto sync on (SK-3), no quiet hours and VIP doesn't break them (NV-23), the terminal's font size 12, 10000 lines of scrollback and a theme that follows the system (ST-3), and the daemon's `--default-agent`. A channel without its own retention reads as the Settings defaults; without its own default agent, the field is unset.
- **Checks:** every section is checked before any is saved. The default agent must be a known agent, the font size 0 or 6–72, scrollback at most 1000000, retention at most 3650 days, long lines Wrap or Scroll, skill patterns and opted-out machines non-empty, and quiet hours both HH:MM or both empty. A `settings.toml` broken by hand shows the defaults (logged) and refuses an update, naming the file.
- **ST-15:** a start that names no agent takes the channel's own default agent, then the one in Settings, then the agent the channel was made with (`session_service.rs` `agent_for`, through `settings_service::default_agent_for`). So once Settings names an agent, it wins over the agent a channel was created with.
- **Follow-ups:** retention isn't carried out yet (nothing archives on these days). The terminal's font, size, theme and scrollback are served but the app and holders don't read them yet. The default agent's model is stored and never passed (SE-2). UpdateChannelSettingsRequest has no `clear_retention`, so a channel's own retention can be changed but not cleared back to Settings. Skills settings could move into `config/` later.
- **Tests:** `daemon/humatad:settings_sections_test`.
- **Reverse:** put `unbuilt_section`/`unbuilt_channel_section` back in `settings_service.rs`, drop `settings_service/sections.rs` and the `agent_for` fallback. The saved files are left as they are.
- **Ruling (Fable, 2026-11-17):** the order stands: the channel's own default agent, then Settings, then the agent the channel was made with. ST-15 names a per-channel override as the only way a channel differs from Settings; the creation-time agent is a fallback, not a pin. No change to `agent_for`. The dialogs that preselect an agent (new session, fork, PR review) must resolve it the same way, so what is shown is what starts.
- **Amendment (2026-11-17):** the daemon tells each channel's agent: `Channel.default_agent` is now what `agent_for` would pick (channel's own, Settings', creation agent), republished when either setting changes. The new session, desk and PR review dialogs preselect it, with Settings' local pick only before the daemon has said; the Settings agent pick is sent to the daemon. Fork keeps the parent's agent.

## D771: provider import previews a whole plan and spends it once (MP-23) (VERIFY worker, 2026-11-17). Provisional
- **Pick:** PreviewProviderImport (user-only, no presence) takes the picked file as UTF-8, or refuses it as INVALID_ARGUMENT `file`. It returns the text as given, its providers, every command each runs (`Provider::commands`), and the ids it replaces (already in the file, or the built-in Lima). It saves nothing. A file with problems (bad TOML, a bad provider, no providers, or a merge that wouldn't parse) returns them with no token. Otherwise its `pl_` token holds the whole providers file the import would write, plus the file's revision at preview. When no imported id is in the file yet, that file is the current text with the import appended, comments kept; otherwise the TOML tables are merged and the named providers replaced. ImportProvider (user, present) spends the token once, within ten minutes. It is FAILED_PRECONDITION `plan_stale` when the token is unknown, spent or old, or when the providers file changed since the preview. Otherwise it writes the file whole and returns the new revision.
- **Not taken:** re-parsing the file at import (the plan must be exactly what was shown); holding plans across a daemon restart (a restart asks for a new preview).
- **Follow-ups:** the idempotency key isn't replayed (a repeat after success is `plan_stale`, and nothing is written twice). The table merge drops the existing file's comments.
- **Tests:** `daemon/humatad:provider_import_test`.
- **Reverse:** return `not_built` from both calls in `provider_service.rs` and drop `provider_service/import.rs`.

## D772: An agent reads machine reachability through `topology_get` from the channel's grid, not the machine list: `online`, `offline`, or `unknown` when the grid has no cell for the machine (TP-1, gap 79, 2026-11-17). Decided
- **Problem:** MCP.md promised `topology_get` (WhoAmI + GetChannel + ListSessions, CHANNEL_READ) with a `reachability` per machine, but the tool was unserved. Reachability lives in `MachineService.ListMachines`, a user/host call an agent's CHANNEL_READ doesn't cover, and on a machine daemon it isn't built (owned by the remote-cp fix).
- **Pick:** `daemon/humatad/src/mcp/topology.rs` also calls `ChannelService.GetGrid` (D764), which is CHANNEL_READ on the same channel. This machine is always `online`. Another machine is `offline` if any of its grid cells is MACHINE_OFFLINE, `online` if it has cells and none is, and `unknown` if it has none (a channel with no repos, or a machine only a session names). Order: this machine, then the channel's machines in order, then any other machine a live session is on. Each lists the channel's live sessions there as `{id, title, agent, status}`. `id` and `name` are both the machine's id as the channel and sessions carry it: a remote machine's name, and the host's own `ma_` id (no channel-scoped call names the host).
- **Not taken:** letting agents call ListMachines (widens what an agent sees past its channel); a new per-machine field on ChannelGrid (a proto change for the one case of a channel with no repos).
- **Follow-ups:** a channel with no repos shows its other machines as `unknown`; if that matters, add per-machine reachability to `ChannelGrid`. Host away: the tool answers from the machine's own daemon, where GetGrid sees every other machine as offline (D764 with no machine list).
- **Tests:** `daemon/humatad:mcp_test` `topology_get_names_this_machine_and_the_channels_sessions_on_it`, `tools_list_offers_only_tools_the_daemon_runs` (now expects it listed); `daemon/humatad:humatad2_unit_test` (`mcp::topology` order and reachability cases).
- **Docs:** `api/MCP.md` topology_get row; `flows/cases/README.md` gap 79; `flows/cases/agents/08-forking-onto-another-machine.md`.
- **Upgrade:** none: no schema or proto change. An older `humatad2 mcp` keeps answering not built until updated.
- **Reverse:** drop `Operation::TopologyGet` from `serves()` and the dispatch, and `mcp/topology.rs`.

## D773: GiveOwnCopy for a context-only fork on another machine is relayed to that machine, which makes the copy as it does for its own sessions; the host shows the machine's steps and then records the fork's new branch and folder (FK-9, VERIFY, 2026-11-17). Decided (by the fixing agent; reversible)
- **Problem:** `session_service/own_copy.rs` answered "not built" for a fork whose record names another machine, although such forks are made there (D187, D208).
- **Pick:** the host runs its own checks first: the access rule, the idempotency key, and that the fork is context-only (the host keeps the fork origin of a fork on a box). It then relays GiveOwnCopy to the machine under the key `host-own-copy-<caller's key>`, so a retry gets the machine's operation back. An offline machine is refused at once with MachineOffline. A machine's refusal (busy, has its own copy, no conversation) goes back at once, before any host operation exists, as a refusal here does. After that the host's GIVE_OWN_COPY operation has the same step keys as the machine's and mirrors them (`archive_remote::follow_to_end`) under the fork's lifecycle lock. On success the host reads the fork from the machine with GetSession. It records each repo's new branch, and the folder and clone paths laid out on its own root, as `place_session` does for a session on a box. Then it publishes the fork and its family.
- **Not taken:** asking the machine and answering from its operation without a host operation (the app follows host operations only); reading the new branch from the machine's pointer reports (they arrive late and may never name a repo with no commits).
- **Upgrade:** the machine's peer link now lets the host relay GiveOwnCopy (`relay.rs` LISTED_METHODS, beside ForkSession), so the machine needs this humatad2 too. A machine on an older humatad2 refuses the call as off its peer list (PermissionDenied) and nothing changes; there is no fallback.
- **Tests:** `daemon/humatad:own_copy_remote_test` (red in run 108 before the fix: the host answered not built); `humatad2_unit_test` (the peer allowlist).
- **Reverse:** put the `not_built_part` answer back in `give_own_copy_of` and drop `own_copy_remote`, `run_own_copy_remote` and `take_up_own_copy`.
- **Follow-up:** a link lost during the copy fails the host's operation while the machine may finish it. The host's record then keeps the old branch and folder, and asking again is refused because the fork has its own copy. The host should read the fork again when the machine's link comes back. An older machine's off-list refusal should come back as a Migrate error naming the machine (the force-upgrade rule), not a bare PermissionDenied.
- **Ruling (Fable, 2026-11-17):** confirmed: the bare PermissionDenied must go. The refusal is FailedPrecondition with kind `peer_version_unsupported` naming the machine and its version, with the fix "Migrate <machine> from the Inbox", and the host makes sure that machine's VIP Migrate (daemon) item exists (D281). It is made once in the relay (`relay.rs`), for any method the host lists and the machine refuses as off its list, not in `own_copy` alone; where the host knows the machine's version from Hello it refuses before relaying. Not `host_needs_migrate` (that is the host lacking a build) and not Unimplemented.
- **Done (2026-11-17):** `machine_failed` maps the refusal for every caller; GiveOwnCopy also posts the Migrate item through `RemoteWork::peer_refused`. The version is not named (the host keeps no peer version per link). Test `own_copy_remote_test`, not seen red.

## D774: SendNote, AskSession and AnswerQuestion are served: a note or question is typed once into the other session's agent after its turn, through the input gate; the asked session's answer is the asking call's result (TP-5; VERIFY worker, 2026-11-17). Provisional
- **Problem:** all three answered `not_built`, so `humata session note|ask` and MCP `session_note`/`session_ask`/`session_answer` failed, though SPEC TP-5, CLI.md and the MCP tool list promise them.
- **Pick:** `session_service/notes.rs`. Both calls gate on SESSION_NOTE with the target session (an agent's call asks the user first, answer_gated). A session can't note or ask itself. The target must run here with its agent Running; else FailedPrecondition, nothing typed (resume it first). Moving (MV-4) refuses as Type does. It waits while the turn is WORKING or NEEDS_INPUT (a note up to 45 s, TP-11's default; an ask until its own deadline), then `await_keys`, then `input_gate::guard` (fails closed, D762), then types the one line and a separate return with holder keyed input (`tp5/<hash of caller + idempotency key>`), so a retried call types nothing twice. An outdated holder is refused with the Migrate fix. The line names the sender (session title and id, "the user" or the script's name), never a path or branch (D743); control characters and line breaks become spaces; empty is Missing, over 16 KiB TooLong.
- **Questions:** `qu_<id>` (from the same hash when keyed, so a retry reuses it; random otherwise) held in the daemon's memory as long as the asking call. `timeout_seconds` 0 means 30 minutes, capped there; MCP asks with 600 s (TP-11's longest wait). Only the asked session's own agent answers (others PermissionDenied); answered, declined, expired or abandoned questions are NotFound, with the fix to send the answer as a note. A repeated answer with the same idempotency key is taken again. A turn that doesn't end before the deadline makes the ask EXPIRED with nothing typed. CLI: `humata session answer <qu_id> [answer] [--decline]`.
- **Not taken:** a durable turn-end queue (SendNote never answers `typed: false`; a note to a session still mid-turn after 45 s is Unavailable and says to send it again); MCP `when` is ignored (always at turn end).
- **Follow-ups:** relay notes and questions to a session on another machine (refused now, FailedPrecondition, nothing typed); questions survive a daemon restart (today the asking call ends with it); a retried ask after its answer waits again rather than replaying the answer; MCP `session_answer` has no decline field.
- **Tests:** `daemon/humatad:session_notes_test` (waits for the turn then types once; an agent's note asks first and names its sender; answered, declined and expired questions; only the asked session answers; a stopped session gets nothing); `cli:dry_run_test` (`session answer`).
- **Upgrade:** none: new behaviour behind existing RPCs; an older daemon still answers `not_built`.
- **Reverse:** put the three `not_built` stubs back in `session_service.rs` and drop `notes.rs`, the MCP arms and `SessionCommand::Answer`.

## D775: `session type` reports agent_busy only while the turn is WORKING or NEEDS_INPUT; PROBABLY_IDLE is not busy (VERIFY, 2026-11-17). Decided (by the fixing agent; reversible)

- **Pick:** `terminal.rs` `agent_busy` was "anything but Idle", so a Copilot agent idle at its prompt (no idle hook heard, output quiet: PROBABLY_IDLE) read busy. Now only WORKING and NEEDS_INPUT are busy. The desk's move and rename guards keep "anything but Idle" (they must be sure).
- **Amendment (VERIFY wave 3, 2026-11-17): typing at the prompt after the trust box was answered on the screen is not busy.** Live, on a fresh session whose trust box was answered and whose screen sat idle at Copilot's prompt, the next `session type` returned `agent_busy: true`: the box's card still held the session's NEEDS_INPUT, as it was withdrawn only at the next quiet spell. No timer was added: the re-read on output that withdraws the card (D767 amendment) also ends that hold. Test: `daemon/humatad:screen_questions_test` `typing_at_the_prompt_after_the_trust_box_was_answered_on_the_screen_is_not_busy`.

## D776: A machine's identity is the random id its own daemon keeps in `~/.humata2/state/machine-id`; its name is a label and its pinned host key a second check, so a different machine under an old name gets nothing of the old one, and the same machine under any name is offered its sessions back (owner's point, Fable ruling, 2026-11-17; amends D61, D333, LD-12; uses D43, D760, D768, D-UPGRADE). Decided; queued for build after VERIFY, not tonight

- **Problem:** the host's id for a machine is its name. `machine_service.rs` `machine_id(&name)` wraps the name, `hosts.json` (`HostEntry`) has no id field, and `Machines::name_free` is the only identity check. D61 said so "until machines get ids of their own". Everything the host keeps about a machine is keyed by that name: skills sync state (`<state>/skills/<machine>.json`), channel membership, sessions, the peer key, the `humata2.<machine>` pin. So remove vm-a, add another box as vm-a, and whatever a removal missed attaches to the wrong machine. D760 fixed two such leaks one at a time; this is the class. Meanwhile each machine's daemon already makes itself a `ma_<uuid>` in `~/.humata2/state/machine-id` (`serve.rs` `machine_id`, exclusive create, D43 item 7), and the host ignores it: Hello sends the name as `expected_machine`. Two ids for one machine is what broke D768's forwarded call ("session … is on machine vm-a, not ma_…").
- **Pick: the proposal, amended in three places.**
  - **The id is the one the machine already has,** `ma_<uuid>` in `~/.humata2/state/machine-id` (0600), not a new `~/.humata2/machine-id`. A second file would be a second id. It is state, not config: it is never under `config/`, never synced and never copied between machines (portable config syncs settings; a synced id is a cloned identity).
  - **The machine's daemon makes it; the host never writes an id's value.** The host reads it over the link at the end of an add (Hello, proved with the enrollment key as today) and records it as `id` in `hosts.json`. From then on Hello's `expected_machine` is that id, and a different answer is `identity_mismatch`. The host may ask a machine to make a new id (the clone case below); it never sets one.
  - **Nothing on the host is keyed by name.** `pb::MachineId` is always `ma_…`. Skills state, peer key, pin label, channel membership, sessions, marks, stop records and journals use the id. The name stays unique among listed machines (`name_free` and D760's `BeingRemoved` stand) because the CLI and people address by it; the host resolves a name to an id at the edge. D768's "the host fills in the link's id" goes: both ends now say the same id.
  - **The id proves nothing alone.** It is not a secret. The proof of a listed machine is still the mutual enrollment-key proof over a host-key-checked SSH connect. The id says which records are this machine's; the host key and the person say whether to believe it.
- **Removal keeps a record.** Remove and Forget keep, per id: the last name, the last pinned fingerprint, when and how it went (removed, forgotten), and its sessions as archived. D760 holds: skills sync state and channel membership are cleared, and the pin is forgotten. The uninstall keeps leaving `state/` on the machine (it removes the tools and the service only), so the id survives a removal. That is now a rule, not an accident.
- **The cases:**
  - **A different machine under an old name:** its daemon has another id, so it is a new machine. Nothing is offered and nothing attaches. The old one's archived sessions show under "vm-a (removed 17 Nov)".
  - **The same machine added again, under any name:** the id matches a removed or forgotten record. The add finishes as any add (fingerprint confirmed again, fresh enrollment key, skills first sync asks again per SK-5), and the offer below is raised. LD-12 changes: a forgotten machine that turns up again is the same machine, not a new one; a session that was rebuilt elsewhere meanwhile is not brought back, and its old folder is listed as left there.
  - **Humata reinstalled on the machine** (tools replaced, or removed and added): `state/` is kept, so same id, same machine.
  - **A wiped home, or a deleted `state/`** (same host key, new or no id, on a listed machine): the link stops with `identity_mismatch` and nothing runs on it. An Inbox item says vm-a no longer has the id Humata added it with, shows both ids and that the host key still matches, and offers: **It's a new machine** (the old id becomes forgotten, its sessions archived with Rebuild on another machine, SL-4, this one included; the name moves to the new id) or **Leave it** (restore the home from a backup and Reconnect works again). The host does not write the old id back: with the state gone, no holder, session folder or enrollment key is there to prove anything, and "same host key" is also true of another account on the same box.
  - **A changed host key, same id:** the SSH connect fails before any id can be read, so the id never excuses the key. An Inbox item shows the pinned and the new fingerprint (TrustNewHostKey, D333's Reverse; D333's remove-and-re-add goes). Once the person trusts the new key, Hello runs: same id and a good proof means the same machine, re-pinned, nothing else changes. Another id falls to the case above, with the item saying the host key changed too.
  - **Same host key, new id, on a fresh add:** a new machine. If a removed record has that fingerprint, the add's result says so in one line (the sessions of "vm-a (removed …)" stay archived) and offers nothing.
  - **Two machines, one id** (a cloned VM, or one machine reached by two addresses): an add whose id is already a listed machine's stops before it commits and asks. **The same machine:** the add is undone, and the listed one may take this address. **A copy:** the host asks the copy's daemon to make a new id; the session records copied with the disk stay on it untouched and are not attached. **Cancel.** A clone that takes over a listed machine's address, with its disk, key and enrollment key, can't be told from the original; nothing here claims to.
- **What the person sees on a re-add:** one Inbox item, "vm-b is a machine you had before", raised when the add commits. It does not hold the add; the machine is usable at once. It carries: the name and address now and the name it had; when it was removed or forgotten; whether the host key is the one pinned before (if not, a warning line, and Bring back asks for presence); each session with title, channel, agent, last activity, whether its folder and holder are still on the machine (looked up now), and dirty files and unpushed commits from the last pointers; the channels it was in. Choices: **Bring back all**, **Choose sessions**, **Start clean**. Bring back unarchives the picked sessions on this id, puts the machine back in their channels, and resumes holders that still run. Start clean leaves them archived under the id; the offer stays on the machine's page and is not raised again.
- **Not taken:**
  - The name as identity (today): the leak class above.
  - The host key as identity: sshd rekeys, VM clones share keys, and two accounts on one box share one.
  - `/etc/machine-id` or a hardware id: Linux-only, shared by clones, and one per box, not per home.
  - An id the host generates and writes over SSH at add: the daemon's id exists already and is made race-free; a host-written one would need the same care and a second file.
  - The host restoring a lost id: it would let a person, or a wrong click, attach old sessions to a box with nothing of theirs.
  - Bringing sessions back without asking: the id is unproven at add time (the enrollment key is new), so the person decides, with the fingerprint in view.
  - Renaming machines: now only a label edit, but not part of this entry.
- **Seen live:** a probe runs tonight (remove vm-a, add a different machine as vm-a, list what of the old one still attaches). Its results will be appended here as a "Seen live" note. Each thing it finds is a test below and, where it is cheap, a name-keyed fix before this is built.
  - Seen live 2026-11-17 (VERIFY wave 3, 0.2 fe9eea4f: vm-a removed, the alias pointed at a fresh user with a fresh daemon, re-added). The two daemons' `ma_` ids differed, but `host ls` keys machines by name only. What of the old vm-a attached to the new one:
    - all 10 old sessions stay listed under the name; after the re-add they show idle on the new machine, which has none of them, and typing into one returns `not_found` (ghost sessions);
    - the old machine's pending Inbox requests stay pending and carry the old id;
    - the old transfer record stays, its destination shown by name.
  - Cleared correctly: the pinned host key (asked again on re-add); the skills sync record (`first_sync` true); the grid cell. Kept correctly: history (`session read` of old sessions works from the host mirror).
  - Not restored: the channel membership was dropped on removal and not offered back on re-add; it had to be added by hand.
  - Tests this adds: a re-added name shows no old session as idle on the new machine; the removed machine's pending Inbox requests are withdrawn at removal. Evidence: VERIFY `evidence/remote/wave3/120`–`131`.
- **Tests (to be written):** a different machine added under a removed name has no sessions, channels, skills state, pins or history of the old one; the same machine added under another name raises the offer with its sessions, and Bring back resumes them; Start clean raises nothing again; a forgotten machine added again is offered, less its rebuilt sessions; a removal leaves `state/machine-id` on the machine; a listed machine answering another id is `identity_mismatch` and runs nothing; "It's a new machine" archives the old sessions and moves the name; a changed host key raises the two-fingerprint item and, trusted, keeps the machine when the id matches; an add of an id already listed stops and asks, and "A copy" leaves the clone with a new id and no sessions; an empty id file is an error, not a new id (D43); a machine agent's forwarded call names its machine by the id the host knows (D768's failure); the migration tests under Upgrade.
- **Upgrade (D-UPGRADE):** a schema change on the host and a Hello change on both ends, so Migrate items, no mixed-version fallback.
  - "Migrate the daemon on This Mac" also bumps the host schema: `hosts.json` gains `id`, and This Mac's own records are rekeyed to its id at once.
  - Each machine's "Migrate the daemon on vm-a" gains a step: read its id over the link and rekey its records from name to id, in one journaled step that a cut-short run finishes at the next start. Until it is done every call to that machine is refused "Migrate vm-a first"; an unreachable machine's item waits, and nothing is looked up by name meanwhile.
  - Two listed machines that turn out to share an id stop the second one's Migrate at the clone choice.
  - Tests: a host with name-keyed records migrates each machine to its id with sessions, channels, skills state and pin intact; an unmigrated machine is refused, not served by name; a migration cut short finishes; a rerun changes nothing.
- **Docs when built:** SPEC LD-10, LD-12 and the Confirm host key card; `api/GRPC.md` §10 and `api/CLI.md` host rows (a machine argument takes a name or an id); OVERVIEW's machines section.
- **Reverse:** before it is built, drop this entry and D61 stands. After the migration there is no cheap way back: returning to names means a second Migrate that rekeys every record by name and drops the removed-machine records. The Inbox offer alone reverses cheaply (never raise it; a re-added machine then starts clean).

## D777: For an agent, a bare `shared`, `shared/` or `shared/<path>` in `humata` file commands is the channel's shared folder on the machine it runs on; a clone's own folder of that name is `./shared` (Fable 5.2 ruling, 2026-11-17; amends D756 and D655 for this one name; reads D462, D755, D743, D768, TP-4). Decided (by Fable)

- **Problem:** on vm-a a Copilot agent ran `humata cp this-mac:shared/x shared/` and got 'there is no folder "shared" on vm-a'. `Ctx::own_clone` (`cli/src/location.rs`) reads every bare path as the agent's clone (D655), and D756 says `name/` always means the clone, so `shared/` was a clone folder that doesn't exist. Yet everything the agent is told uses the bare word for the channel's folder: D755's fix says "put the file in shared/ here", the `OutsideHumataFolders` message says "a path inside the session's clone or shared/", and the MCP tools say "your own clone or shared/" and take `root: "shared"` with the machine defaulting to the agent's own. An agent that follows the hint as written fails. The same happens on This Mac (one CLI, one code path); `flows/cases/results/copilot-files.md` shows an agent there typing `humata cp shared/agent-note.txt vm-a` too.
- **Pick (option A):** for an agent principal, a location with no `:` whose first path part is exactly `shared` (`shared`, `shared/`, `shared/a/b`) is `Root::ChannelShared` of the agent's own channel (`this_channel`), with no machine named, as a bare clone path is sent today, so the daemon the agent talks to fills in its own machine and D768's forwarding is unchanged. The rest of the path follows as given. It holds wherever `Ctx::location` is used (cp, mv, and the `files` commands), for sources and destinations alike, and on This Mac and a machine's own daemon the same.
  - **The clone's own `shared`:** `./shared` and `./shared/<path>` are the clone, always. A leading `./` is the one way to say "the clone, whatever the name". D756's "`./name` always means the clone" stands; its "`name/` always means the clone" now has this one exception.
  - **The last word `shared` with no slash:** D756's rule 1 stays first (a machine the agent can see with that name is that machine); otherwise it is the shared folder, not `NotAMachineHere`. A hidden machine of that name changes nothing, so nothing leaks.
  - **A wrong guess must be fixable from the error:** when a bare `shared…` location is answered not found (a missing source, or a missing folder under it), the CLI's message says it looked in the channel's shared folder on the agent's machine and that a folder named shared in its own clone is written `./shared/…`. No path is printed (D743).
  - **The person's reading is unchanged:** a bare location from the user is still refused with the expected forms.
  - **Access is unchanged:** `write_place` already allows `Root::Shared` on the agent's own machine (IN-17, D755). Another machine's `shared/` still needs the `<machine>:` form and is still refused for writes with D755's pull hint.
- **Why:** an agent told "shared/ here" types `shared/`; no agent types `<its-own-machine>:shared/x` unprompted, and on a machine's own daemon it would first need `humata host ls` to learn its own name. D743 says agents aren't told paths, so the short names they are given must work as given. The MCP tools already make `shared` a root of its own with the machine defaulted; the CLI now agrees with them. The cost is one reserved top-level name in a clone, with `./shared` as the way round it and an error that says so.
- **Not taken:**
  - **B, keep D756 and reword every hint to `<own-machine>:shared/x`:** it makes the messages longer, makes the agent look its own machine's name up, leaves CLI and MCP reading "shared" differently, and the natural spelling keeps failing with a message about a clone folder.
  - **Look first, then choose** (the clone's `shared/` when it exists, else the channel's): the same command would mean different places in different repos, and a write could land in a repo's tracked folder by surprise.
  - **A new spelling** (`@shared/`, `shared:`): nothing the agent is told uses it, and `shared:` reads as a machine.
  - **Reserving `channel/`, `received/` and `main/` the same way:** an agent can't write those, and no hint names them bare. Only `shared` is given to agents as a bare word.
  - **Refusing a machine named `shared`:** rule 1 keeps such a machine reachable as a bare last word, and `shared/` with the slash is never a machine, so there is no clash to forbid.
- **Build:**
  - `cli/src/location.rs`: in `location`, the agent's no-`:` arm strips nothing and checks the first part: `shared` goes to a new private `own_shared` (ChannelShared, `this_channel`, `machine: None`, the rest as path, trailing `/` trimmed); anything else, and anything starting `./`, goes to `own_clone` as now. Update the module and `own_clone` comments.
  - `cli/src/commands/files.rs` `machine_only`: for an agent, after `NoMatch`, the word `shared` returns `Ok(None)` (falls to `location`). Update its comment.
  - `cli/src/error.rs`: the not-found hint for a bare `shared…` location (a variant carrying the typed text, with the daemon's refusal as its source); `NotAMachineHere`'s text is unchanged.
  - `daemon/access/src/verdict.rs` (`OutsideHumataFolders`) and `daemon/humatad/src/status.rs` (D755's fix) keep their bare "shared/": it is now true as written. No daemon logic changes, unless the test below on a machine's own daemon shows a ChannelShared location with no machine isn't filled in as a clone's is; then fix it where the clone's is filled.
  - Docs: `api/CLI.md` §5.5 ("Inside a session, a bare relative path is the agent's own clone, except `shared/…`, the channel's shared folder on its machine; `./shared` is the clone's"), and the machine-word paragraph; `flows/cases/macro/12-files-across-sessions-and-machines.md` gains the pull case as typed tonight; `flows/cases/results/copilot-files.md` rerun note; OVERVIEW's files section.
- **Tests:**
  - `cli/tests/files.rs` (agent on ch-1, fake daemon): a destination `shared/` sends `Root::ChannelShared` of ch-1 with no machine and an empty path; `shared/notes` sends path `notes`; a source `shared/x.txt` is ChannelShared with path `x.txt`; `./shared/` and `./shared/x.txt` are the agent's clone with path `shared` and `shared/x.txt`; the last word `shared` with no such machine sends to the shared folder, and with a visible machine named `shared` sends to that machine; `sharedstuff/` and `my/shared/` are still the clone; a user's bare `shared/` is still exit 2 with nothing started; a not-found answer for a bare `shared/x` exits not-found and its hint differs from the one for `./shared/x` (assert on the error's kind and that the two differ, not on wording).
  - `daemon/humatad/tests/agent_send_machine.rs`: an agent on box-1 pulls `this-mac:shared/x` into `shared/` and the file is in the channel's shared folder on box-1, not in its clone; an agent on This Mac does the same from box-1; an agent's `cp ./note.txt shared/` on box-1 lands in box-1's shared folder.
  - `cli:this_machine_test` stays green (D768's `this-mac` reading is untouched).
- **Upgrade (D-UPGRADE):** CLI only; no proto, schema or daemon contract change, so no Migrate item beyond the CLI's own. An older CLI keeps the clone reading until it is replaced. A script that relied on a bare `shared/` meaning a clone folder now writes to the channel's folder: none is known, and the fix is `./shared/`.
- **Reverse:** drop the `shared` arm in `location` and in `machine_only`, and take option B: reword D755's fix and `OutsideHumataFolders` to name `<machine>:shared/<path>`.
- **Done (2026-11-17):** CLI `location`/`own_shared`, `machine_only` and `CliError::SharedNotFound` (cp/mv) built; docs (CLI.md 5.5, OVERVIEW, flow 12, copilot-files rerun note) updated. Red run 116; green runs 118 (cli:files_test, cli:this_machine_test) and 120 (agent_send_machine_test). D777b (2026-11-17): the shared-folder hint now follows the daemon's refusal, not the typed arguments. A missing channel or session root is refused as `ENTITY_KIND_ROOT_FOLDER`, and only that gets the hint, so a missing source names itself. Red run 126; green runs 130 (cli:files_test) and 131 (transfers_test, agent_send_machine_test, file_ops_test, humatad2_unit_test).

## D778: Errors name a machine by the name the app shows it under, never its `ma_` id; an UNAVAILABLE the daemon worded is its answer, not the machine offline; a Desk start checks its agent first (bug 8, VERIFY, 2026-11-17). Decided (by the fixing agent; reversible)
- **Names:** `clients/src/machine_names.rs` keeps one process-wide map, machine id to shown name. The live client fills it as it learns names: on connect the daemon's own machine is "This Mac", and every listed machine gets its listing name, or "This Mac" when `is_this_mac` is set (`clients/live/src/map.rs`). The fixtures fill it too. Every error that names a machine (`DaemonError::Offline`, `MigrationNeeded`, `FileError::Offline`, `DiskFull`, `PrError::SignedOut`, `RateLimited`) formats it through `name_of`. A machine not yet listed reads as "That machine". Errors still carry the `MachineId` the app acts on.
  - Why a global: the errors' `Display` is all the UI ever shows, and threading names into every error type and every call site is a large, wide change. A registry that the listing fills is enough because no error is shown before the first listing.
  - Reverse: give the errors a `name` field set where they are made.
- **UNAVAILABLE:** `status_error` (`clients/live/src/connection.rs`) maps UNAVAILABLE to `Offline` only when the status has no `humata.v1.Error` message, or has the kinds MachineOffline or HostUnreachable. A worded UNAVAILABLE becomes `Refused` carrying the daemon's words. This matches the daemon's own rule (`remote_work.rs` `call_failed`).
- **Agent check:** a Desk start, plain or channel, runs `agent_gate::ready` on its local path before it makes anything, as session starts already do. A missing CLI is FailedPrecondition, naming the agent and the program, with a fix. The Missing text now reads "... isn't installed on this machine, or Humata can't find it: `<cli>` was not found".

## D780
- **Decision (0.2.1 fix lane, 2026-11-17): the daemon reads the login shell's PATH, gated by the launch agent setting an environment variable.** launchd starts `humatad2 serve` with `/usr/bin:/bin:/usr/sbin:/sbin`, so gh, copilot and codex (Homebrew) and claude (`~/.local/bin`) weren't found. `daemon/humatad/src/tool_path.rs` widens PATH once in `main`, before the runtime starts any thread: first the login shell's PATH (`$SHELL -l -i -c`, 5 s timeout, read after a marker so a profile banner is ignored), then the PATH it inherited, then `/opt/homebrew/bin`, `/opt/homebrew/sbin`, `/usr/local/bin`, `~/.local/bin`, `~/bin`, each folder once. Every caller, and the tabs the daemon starts, inherit it. One `gh_program` lives in `tool_path.rs` now, in place of three copies.
- **Why an env var (`HUMATA2_LOGIN_PATH=1` in the plist's EnvironmentVariables) and not a flag:** the installer reloads a changed plist before the staged daemon is swapped in, and an older daemon would refuse an unknown flag and fail to start. Test daemons don't set it, so they keep the PATH their tests give them.
- **Left out:** remote machines (`serve --stdio` over SSH, `bridge`) keep the PATH they inherit. host_probe already looks in `~/.local/bin` there.
- **Also:** This Mac's GetMachine reads its own health (`host_probe::read(false)`, no folder sizes; the Storage page has them), where it used to send none. A host page waiting on Migrate shows no load or Scripts error of its own (`MachinePageState::shown_load_error`/`shown_scripts_error`), since the read-only banner already says why.
- **Reverse:** drop the plist key and the `tool_path::asked()` call in `main`.

## D779: The main window opens with titlebar options (transparent, no title, the system's traffic-light place) instead of none, and View has Enter Full Screen on ⌃⌘F; text zoom moves to 0.2.3 (owner's 0.2.1 bugs 3 and 4, 2026-11-17). Decided (by the fixing agent; reversible)
- **Why:** on macOS, GPUI (`gpui-pre-macos` 0.3.12 `window.rs`, `MacWindow::open`) gives a window with `titlebar: None` only `Titled | FullSizeContentView`: no Resizable, Miniaturizable or Closable style, so it can't be resized, zoomed or made full screen. With `TitlebarOptions { appears_transparent: true, title: None, traffic_light_position: None }` it gets those styles and the same transparent, title-hidden, full-size-content look.
- **Full screen:** a `ToggleFullScreen` action in `desktop/src/main.rs`, bound to ⌃⌘F and listed in View as "Enter Full Screen". If macOS also adds its own Enter Full Screen item to View, drop ours; check on the installed build.
- **Zoom (bug 4), deferred to 0.2.3:** GPUI has no user scale factor (`set_scale_factor` is test-only) and `rem_size` only reaches rem-based sizes. ui/src has about 400 `text_size(px(..))` calls and about 2,400 `px(..)` in all, so an app-wide zoom means moving them to rem units, or a scale helper, with the goldens kept equal at 100%. That's roughly a day or two for one lane. Also, ⌘0 already toggles the sidebar, so the zoom reset needs a decision: move ToggleSidebar, or reset zoom on another key.
- **Reverse:** set `titlebar: None` again in `main_window_options`, and remove the action and its menu item.

## D781: An Inbox item read off an agent's screen opens with that session's live terminal under it and a Dismiss button below; the terminal never takes focus by itself, never resizes the program, and Dismiss never presses a key (owner's request, Fable 5.2 ruling, 2026-11-17; amends D767, uses D271 point 3, D750, D762, IN-9). Decided (by Fable), being built now. Superseded by D845 (INBOXHOOKS)
- **Problem:** a question an agent asks on its own screen (Copilot's "Confirm folder trust", D767) reaches the Inbox as a card with parsed choices. The owner wants to see the real dialog and answer it there, as in the agent's TUI, then close the item by hand: "can we just render the terminal for me to approve or deny and then I can press a 'dismiss' button below it once I have done that?"
- **Scope:** only items raised from a dialog that is on the screen now (`screen_ask`, and any dialog `screen_question::asked_on` learns later). A permission ask held by a hook (D191) and Copilot's `ask_user` form held by its hook (D761) have no dialog on the screen while the card waits, since the agent is waiting in the hook, so they keep today's card and get no terminal. Main's shape listed them; this narrows it.
- **Pick, the marker:** `InboxRequest` gets `ScreenAsk on_screen = 22` with `uint32 tab` (the agent tab's index, as `Daemon::link` numbers it). `screen_ask::raise` sets it; nothing else does. The client model's `Request` gets `on_screen: Option<usize>`. The app decides everything below from this field, never from the card's words.
- **Pick, the Inbox detail (wide):** top to bottom, 12 pt apart:
  1. The card as today, at most 560 pt wide: title, requester, place and Open session ↗, the question and folder, the choice buttons, the note that Humata never answers for you. Deny and Reply… are not offered.
  2. The terminal, as wide as the detail pane: the session's agent tab, live, through the same `Daemon::link(session, tab)` a session tab uses, drawn by the same `terminal_replica` and `terminal_view`. It is a controller like any other attach (D14, D271 point 3), so keys typed in it reach the program.
  3. One row under it: on the left, in secondary text, "Answer in the terminal or pick a choice above. Dismiss closes this item and presses nothing."; on the right, the Dismiss button.
- **Call 1, the choice buttons stay**, above the terminal. They are the one-click path and the only path when the terminal can't be shown; the terminal shows the pick land. A pick still goes through D767's re-read, so it presses nothing on a screen that stopped asking.
- **Call 2, remote sessions need nothing new.** `link` goes to the host, which relays `TerminalService/Attach` to the machine's daemon and its holder (`relay.rs`), as session tabs do today. The card, the pick and the withdrawal already run on the machine's daemon (D767's amendment). A machine that can't be reached is the "can't show" state below.
- **Call 3, no terminal without a waiting dialog.** The terminal is drawn only while the item waits. An item in Answered or All shows its card and how it ended, nothing more. A waiting item whose link fails or ends (machine offline, holder gone, tab replaced) shows one line where the terminal would be: why, in the link's own words, and "Open session ↗ to try there"; the choices and Dismiss stay, and the hint drops the terminal: "Pick a choice above. Dismiss closes this item and presses nothing." *(Amended 2026-11-17, Fable's ruling on frame 116.)* An ended session normally never gets here: the agent's exit withdraws the item.
- **Call 4, Dismiss is D767's Deny, renamed, plus one look at the screen.** The app sends the existing deny with no reply (`AnswerRequest`); no new RPC. For an `on_screen` item the daemon then reads the screen once and types nothing either way:
  - The screen no longer asks: the item is withdrawn as `WITHDRAW_REASON_RESOLVED` and history reads "Answered in its terminal". The lane's re-read withdraws with the same reason when it sees the dialog gone first. This replaces D767's CALL_ENDED for that case; agent exit and a new holder stay CALL_ENDED.
  - The screen still asks: the item is saved as denied, shown as "Dismissed; the question is still on its screen". The same screen isn't raised again (D767). The session stays NEEDS_INPUT until a look sees the dialog gone: `screen_ask` keeps its `Open` entry and the `Asking` hold with no card. Today a deny lets go of `Asking` while the agent is still blocked; that is fixed here, and D782 uses the same card-less hold.
  - The screen can't be read: denied, as above.
- **Call 5, focus and keys.**
  - The terminal never takes keyboard focus by itself: not when the item opens, not when the Inbox selects its first item, not when a new item arrives. Enter alone accepts a trust box, so focus that moves under a key already in flight would answer for the person, against D762 and D767. A click in the terminal focuses it, and so does Tab from the list. Focus shows as a 2 pt accent border.
  - Focused, every key goes to the program, Esc included, exactly as in a session tab (`ShortcutPolicy::default`). So Esc picks Copilot's "No (Esc)" and does not leave or dismiss anything in Humata. Command shortcuts still belong to the app. A click outside the terminal gives focus back.
  - Unfocused, the Inbox's keys work as today. Dismiss is a button only; it has no key.
  - When the item is withdrawn while the terminal has focus, focus goes back to the list, so keys never fall through to the next item's terminal.
- **Size (D271 point 3, D750):** this view never sizes the program. `FitPolicy` gets `Never`: no Resize frame is ever sent, focused or not, and the view draws the grid at whatever size the session's own tab set. The font shrinks from the terminal's size until the grid's columns fit the box, down to 9 pt; a wider grid scrolls sideways. The box shows whole rows only and opens scrolled to the bottom rows, where agents draw their dialogs. Its height, border and 8 pt inset included (the inset shows on all four sides), is at most 420 pt and at most what the detail has left after the card and the hint row, so Dismiss is always in view; it never shows fewer than 8 rows (or the grid's, if fewer), and below that the detail scrolls instead. *(Amended 2026-11-17, Fable's ruling on frames 114–116.)*
- **One at a time:** only the open item's terminal is linked. Selecting another item, leaving the Inbox or the item ending drops the link; the program keeps running and live holders are untouched. It is a separate replica from the session tab's, so both can show at once and the tab's writer count (SE-24) counts it.
- **The session's Inbox panel** (narrow, IN-9) shows no terminal, since the terminal is beside it. Its card offers Dismiss in place of Deny for these items.
- **Not taken:**
  - Focusing the terminal on open: one stray Enter would grant folder trust.
  - A read-only terminal with buttons only: the owner asked to answer in the terminal.
  - Dropping the choice buttons: they still work when the terminal can't be shown, and from the CLI.
  - A new Dismiss RPC or answer kind: the deny already types nothing; only its history and the status hold were wrong.
  - Dismiss pressing Esc for the person: Humata never picks a choice by itself.
  - Fitting the program to the Inbox pane: two views would take turns sizing it and the agent would redraw its dialog under the person.
  - Terminals for hook-held asks: their screen shows a waiting agent, not a dialog.
- **Design goldens (D76), under `desktop/tests/__snapshots__/design/`, on the fixture's Inbox at the frame size of `26-inbox`:**
  - `114-inbox-screen-question.png` (light): the list on the left with the Copilot trust item selected. Detail: the card (title "Copilot asks: Do you trust the files in this folder?", the folder, three choice buttons "Yes", "Yes, and remember this folder for future sessions", "No", the never-answers note); under it the terminal box, full pane width, 8 pt corner, 1 pt line border, 8 pt inside, terminal background, showing a fixture screen of 100 columns by 24 rows with Copilot's boxed trust dialog at the bottom and the highlight on row 1; under it the hint on the left and the kit's secondary button, 24 pt, for Dismiss on the right. Not focused. *(Amended 2026-11-17, Fable's ruling: the card matches the request card, 12 pt inside, 8 pt apart, orange border; the box shows the screen's last 21 whole rows at 375 pt; list titles end in an ellipsis and the times keep one right inset.)*
  - `115-inbox-screen-question-dark.png`: the same in dark with the terminal focused: 2 pt accent border, cursor drawn right after "❯ 1. Yes" in place of one space, so the dialog's right edge stays in its column.
  - `116-inbox-screen-question-no-terminal.png` (light): the same item on an offline machine: the card and choices, the one-line reason with Open session ↗ in place of the terminal, call 3's hint without the terminal, and Dismiss.
  - The builder renders these headlessly from `desktop/tests/design.rs` and sends them to Fable for approval before the screen is matched to them.
- **Tests:**
  - `terminal/client:component_test`: a `Never` replica sends no Resize when shown, focused or when its bounds change, and draws a snapshot larger than its bounds.
  - `features/inbox`: an `on_screen` waiting item offers its choices and Dismiss, not Deny or Reply; Dismiss sends one deny with no reply; the terminal to show is the selection's session and tab only while it waits; an answered or withdrawn one gives none.
  - `daemon/humatad:screen_questions_test`: a screen question's item carries its tab; dismissing while the screen asks types nothing, keeps the session NEEDS_INPUT and raises no second item, and the status clears once the dialog goes; dismissing after the dialog was answered in the terminal withdraws the item as resolved and types nothing.
  - `desktop` (headless): opening the item leaves focus in the list, and a key pressed then reaches no terminal; after a click in the terminal, Esc reaches the program and the Inbox stays open; the item being withdrawn returns focus to the list; a remote session's item shows its screen through the fake's link.
  - The three design frames.
- **Docs:** `api/GRPC.md` (InboxRequest `on_screen`, the deny of a screen item), `api/CLI.md` `humata inbox deny` row, OVERVIEW's Inbox section, SPEC IN-9.
- **Upgrade:** one optional proto field and one enum value used in a new place; no schema change. A machine's daemon from before this build sets no `on_screen`, so its items show today's card with Deny and no terminal until its Migrate item is done. A journaled item made before the upgrade has no tab and stays a plain card.
- **Reverse:** stop setting `on_screen` in `screen_ask::raise`. Every item is then today's card; `FitPolicy::Never` and the card-less hold can stay.
- **Built (uncommitted, 2026-11-17):** as picked, with three changes. The Inbox terminal links through `Terminals::link_unsized` (the same attach RPC, at no size, which the daemon doesn't resize), because the live `link` attaches at 80x24 and would size the program. A Dismissed card's task and D782's `hold_without_card` both run `screen_ask::hold_card_less`, the one card-less hold. Frames 114–116 are `CANDIDATES` in `desktop/tests/design.rs`, drawn into the test's outputs and not checked, until Fable approves them and they move into `FRAMES`.

## D782: Whether an agent's own approval dialogs come to the Inbox is a switch per agent, on by default; it replaces D199's one switch and never touches the approvals Humata itself asks for (owner's request, Fable 5.2 ruling, 2026-11-17; amends D199, uses D720, D725, D761, D763, D767, D781). Decided (by Fable), being built now. Amended by D845: screen dialogs are always terminal-only
- **Problem:** the owner: "I want an option to be able to disable things from coming into my inbox if it's a request from copilot or claude code or codex for approval. Like what if I only want approvals for things that I control programmatically". D199 already has this as one switch for every agent (Settings › Agents › "Ask for permission in the Inbox"), made single only because the Settings frame of the day had no per-agent rows. D725 has since given Settings one row per agent.
- **Pick:** `[agents.<kind>]` in `~/.humata2/config/agents.toml` (D720) gets `asks_in_inbox`, default true. Settings › Agents shows it as an indented row under each enabled agent, beside "Install when missing": "Ask in the Inbox", with "Off: it asks in its own terminal" (`settings-agent-asks-<id>`). Custom kinds get the row too; the plain shell has none. D199's single switch and its row are removed, so there is one control, not two.
- **What off covers, for that agent's sessions:** everything the agent itself would have asked in its TUI:
  - its permission asks (the permission hook answers at once with no decision, so its own prompt asks; D191, D199);
  - its ask-user form held by a hook (D761: empty output, so Copilot's form opens);
  - dialogs read off its screen (D767, D781: no item is raised).
- **What it never covers:** the approvals Humata's own rules raise, which are the ones the owner controls programmatically: agent-access asks (Ask rules, `RequestAccess`), an agent's `AskUser` through Humata's MCP or CLI, file and cross-machine copy approvals, board and repo proposals, Desk moves, Migrate items, and D763's unsent first message (that item is about Humata's own message).
- **Nothing is hidden:** with the switch off, a dialog `screen_ask` recognises still holds the session NEEDS_INPUT with no item, through D781's card-less hold, until a look sees it gone. Permission prompts the screen read doesn't recognise show needs-input as the agent's own notices report it, as under D199 today.
- **Call 1, the default is on.** That is today's behaviour, and a person who never opens Settings must not have agents waiting unseen.
- **Call 2, no per-session override.** The choice is about which agent's prompts a person trusts to read in a terminal; a third level would need its own control, status and history for no asked-for case. A session already follows a change at its next ask, with no restart.
- **Call 3, a held first message (D763) still waits.** The input gate (D762) reads the screen, not the Inbox, so it holds the text while the dialog shows whether or not an item exists. It is released as today: the person answers in the terminal, the gate reads a clear input, the text is typed and its return pressed. After 30 min unsent, D763's own item is still raised, since it is Humata's.
- **Call 4, D761 counts as the agent's dialog.** Copilot's `ask_user` is its own form, caught by a hook; off means the form opens in its terminal. An agent that asks through Humata's `AskUser` chose Humata's Inbox, and that is never switched off here.
- **Call 5, where:** the Agents card (D725), per agent, as above. Not in Notifications: this decides whether an item exists, not whether it alerts.
- **Where it is read:** by the daemon that runs the session, at each ask, from its own `agents.toml`: `Inbox::asks_in_agent` takes the agent kind; its callers are the permission hook and question hook (`hook_of`) and `screen_ask::raise`. A missing or broken file reads as on (D720's rule). Already-held items stay held when the switch goes off (D199).
- **Remote machines, owed check:** hooks and screen reads run on the machine's own daemon, so the machine needs the host's `agents.toml`. The builder confirms that the config sync carries `agents.toml` to a machine before its next ask and, if it does not, has the host push the file on every change. Until a machine has it, that machine asks in the Inbox (the default).
- **Compatibility:** `Settings.agents.permissions_in_agent` stays in the proto. Reading it is true only when every kind is off; writing it sets every kind. `AgentDefinition` gets `asks_in_inbox`, and the call that sets "Install when missing" takes it (`SwitchChange.asks_in_inbox`).
- **Not taken:**
  - Keeping D199's global switch above the per-agent ones: two controls for one thing, and no way to say which wins.
  - A per-session or per-channel override (call 2).
  - Default off: agents would block with nothing in the Inbox for people who never changed anything.
  - Hiding the item but still holding the hook: the agent would wait an hour on an answer nobody can give.
  - Switching off Humata's own asks here: those have Agent access rules (IN-17), set per capability.
- **Design golden:** `117-settings-agents-ask-in-inbox.png` (light): Settings › Agents with the Agents card: Claude Code, Codex, Copilot, OpenCode, each on, each with its indented "Ask in the Inbox" row; Copilot's is off and shows the subtitle; the old single switch card is gone. If a frame already draws the D725 card, it is redrawn instead of adding 117. Sent to Fable before the screen is matched. Fable accepted 2026-11-17: it matches frame 29's card, rows and switches and keeps the chrome quiet. The accepted capture is `native/117-…png` (`native_test` `the_ask_in_the_inbox_rows_match_frame_117`), and `design/agent_asks.rs` draws the same page as design frame 117.
- **Tests:**
  - `daemon/humatad:hooks_test`: with Copilot off and Claude Code on, Copilot's permission hook and question hook return no decision and raise nothing, Claude Code's raise their items, and an access ask and an `AskUser` from a Copilot session still raise theirs.
  - `daemon/humatad:screen_questions_test`: a switched-off agent's trust dialog raises no item, the session is NEEDS_INPUT while it shows and not after, and a held first message is sent once the dialog is answered in the terminal.
  - `daemon/humatad` agent switches test: the key defaults to on, a save keeps the file's other lines, a broken file reads as on, and the old `agents.permissions_in_agent = true` becomes every kind off.
  - `features/settings`: toggling a row sends that agent's change, a refused save puts the switch back with the error, and the row hides while the agent is off.
  - The design frame.
- **Docs:** `api/GRPC.md` and `api/CLI.md` settings rows, OVERVIEW's Permission controls line, SPEC ST and IN-17 notes.
- **Upgrade:** at its start the daemon reads the store key `agents.permissions_in_agent`; if true it writes `asks_in_inbox = false` for every kind in `agents.toml`, then deletes the key. No schema change; one optional proto field. A machine's daemon from before this build keeps D199's single key until its Migrate item is done.
- **Reverse:** drop the `asks_in_inbox` rows and read every kind as on; restore D199's row from `ui/src/settings.rs` history.

## D783: Removing a machine archives its sessions as lost with it and withdraws its pending Inbox requests, so a machine added later under the same name takes on none of them (VERIFY finding, 2026-11-17; the name-keyed fix D776 asks for; uses D61, D760, LD-12). Decided, built
- **Seen:** vm-a was removed, the same ssh alias was pointed at a fresh machine, and that machine was added as vm-a. All 10 old sessions stayed listed and showed idle on the new machine. Typing into one returned `not_found` from the new machine's daemon. The removed machine's file, session and question cards stayed pending.
- **Cause:** the host's id for a machine is its name (D61). Removal took the machine out of channels (D760) but kept its sessions. With the new vm-a linked and no view of the session, `RemoteWork::merge_reported` left the host's own status, which is Idle. Type relayed by `record.machine` to the new daemon, which never had the session. Inbox entries keyed by the session or by `journal.machine` stayed pending.
- **Decided:** when a removal commits, `Work::removed` does three things, after D760's channel step, each failure logged with a fix and none failing the removal:
  - archives every session on the machine as `machine_lost`, the same way Forget machine does (`Forgetting::archive_lost`, now passed a WithdrawReason). This ends grants, withdraws the sessions' requests and lets go of the host's reports.
  - withdraws every pending request the machine's journal brought (`Inbox::withdraw_pending_from`, new `inbox/removed.rs`), for requests not tied to a recorded session.
  - uses the new reason `WITHDRAW_REASON_MACHINE_REMOVED = 12` ("the machine was removed").
- **The mark:** `ArchiveSummary.machine_removed` (new field 5) goes on every archived session that ran on the machine when it is removed or forgotten (`mark_machine_removed` in `archive_lost`), including sessions archived before the removal. The refusal is keyed on this mark, not on how the session was archived. A session started on the new machine has no mark. A rebuild removes the archive row, so the mark goes with it.
- **Input:** Type, the queued-typing relays and Attach, read-only or not, refuse a marked session (or one that is `machine_lost`) with FailedPrecondition, from `terminal/lost.rs`. The message says the machine was removed or forgotten. The fix says `humata session read <id>` still works, that new work goes in a new session, and that a machine of the same name is another machine.
- **Restore:** RestoreSession refuses a marked (or `machine_lost`) session with FailedPrecondition and the same fix (`ArchiveError::MachineGone`), so the new machine is never asked to undo an archive it never made.
- **Migrate and journal-only requests:** the machine's Migrate items are withdrawn with MachineRemoved (`Migrations::machine_removed` through `Watch::removed`) and dropped from the pending map, so the new machine gets its own items. Removal and Forget machine both run this, and the journal-only withdrawal, through `Work::withdraw_removed`.
- **Follow-ups:** the remote-archive and restore follow-ups kept for the machine (`archive_remote/pending`) are deleted when its sessions are archived as lost (`pending::remove_of`), so a restart doesn't wait for the next machine of the name. A follow-up running now keeps one watch of the machine's link for its whole wait (`next_up`), so it ends when the link is forgotten and never wakes for a machine added later under the name. Before this, it subscribed afresh on each wait and could pick up the new machine's link. The calls it made were keyed by the old session and operation ids, so a fresh machine would have answered `not_found` or released nothing, but they went to the wrong machine.
- **Free timers:** checked, nothing to fix. The host's `free_folder` for a session on another machine only marks the row freed and never calls that machine (`elsewhere`), so no timer reaches the new machine.
- **Fits D776:** removal now archives a machine's sessions and marks them `machine_removed`. When D776's "Bring back" runs because the same machine (by ma_ id) returns, it must un-archive those sessions and clear the mark.
- **History:** the host's mirror is untouched; `session read` keeps working.
- **Not taken:** D776's ma_ identity work (still deferred); deleting the sessions (history would go); hiding them without archiving (Type would still relay).
- **Open:**
  - Settled by D784: Remove machine, while the machine still answers, ends each of its sessions there through the machine's own StopSession (`Forgetting::end_sessions_on`, called once from `remove_machine` before the link goes down). D784's lane makes that path end the holder and its process group. Forget machine ends nothing, and there is no start-up sweep. A session the machine doesn't end within 30 seconds is logged and archived all the same.
  - A follow-up started after the link was forgotten (an archive cut by the removal itself) subscribes to the new name's link. It only reads, because the host's row already exists by then.
  - Channel views are told about marked sessions only through the session feed.
  - The summary's lost-work list is usually empty, because `forget_peer` drops reports when RemoveMachine starts.
  - There is a short window between `Machines::remove` freeing the name and the archive step.
  - Channel membership isn't offered back (D776).
- **Tests:**
  - `daemon/humatad:forget_machine_test` `a_machine_added_again_under_a_removed_ones_name_takes_on_none_of_its_sessions_or_requests`: a live session and one archived first, both checked for list, status, Type, and Attach on the live one, plus the card's withdrawal and the history.
  - `daemon/humatad:forget_machine_test` `a_session_on_the_machine_added_again_takes_typing`.
  - `daemon/humatad:forget_machine_test` `removing_a_reachable_machine_ends_its_sessions_there` (D784).
  - The same rig test refuses Restore of the session archived first.
  - `daemon/humatad:upgrade_inbox_test` `removing_a_machine_withdraws_its_migrate_item` and `forgetting_a_machine_withdraws_its_migrate_item`.
  - `daemon/humatad:humatad2_unit_test`:
    - `upgrade::items::tests::a_removed_machines_items_are_withdrawn_and_another_machines_stay`;
    - `session_service::archive_remote::tests::a_follow_up_ends_with_its_machine_and_never_wakes_for_the_next_of_its_name`;
    - `session_service::archive_remote::pending::removed_tests::a_removed_machines_follow_ups_go_and_another_machines_stay`.
- **Docs:** `api/GRPC.md` section 7 note after REMOVE_MACHINE; inbox.proto and sessions.proto comments.
- **Reverse:** drop the `end_removed_sessions` call from `remove_machine`, the `archive_removed` and `upgrade.removed` calls from `Work::removed`, the mark in `archive_lost`, the `refuse_if_machine_gone` calls, the Restore refusal, `withdraw_removed` in `finish_forget`, and `drop_follow_ups_of`. Proto values (WithdrawReason 12, ArchiveSummary 5) stay reserved.

## D784: Every running holder of ours is listed on its machine, attachable, adoptable when no channel has it, and ended only when the user says so (owner's report of leaked Copilot sessions, owner's redirect and Fable 5.2 ruling, 2026-11-17; settles D783's open item on removed machines' holders; uses D264, D292, D776). Decided (by Fable), being built now

- **Owner's words:** "If the terminal is meant to live let it live. I want to be able to discover the terminals + copilots and I also want to be able to explicitly kill them or not… account for them and resume and also sometimes just call it quits."
- **What leaked:** the agents the owner killed were all left by test harnesses. Holders SIGKILLed by kill-matrix and `kill_holders` left their programs running, since only a holder that ends cleanly hangs its program up. `return_after_text` never ended its holder. `codex_resume.py` left its codex app-server. VERIFY lanes had no teardown. `end_by_pid` ended the holder but never its program's process group.
- **No start-up sweep.** Nothing ends a live terminal unless the user says so.
- **Discovery:** on each machine the daemon lists every running holder of ours (our holder binary, socket in our own state dir; never the terminal lab host or another state dir), local and remote through the peer relay. Each entry carries its agent (Copilot, Claude, Codex or shell), pid, folder, uptime and whether it is attached now. Holders no session names are listed as "Not in a channel", never hidden.
- **Every listed session attaches:** one whose holder is gone is revived or reads as ended, never a running row that refuses Attach.
- **End:** an explicit "End session" ends the holder and the agent's whole process group. One confirmation names the agent and says it can't be undone. UI and CLI (`humata sessions end`); agent callers sit behind the destroy switch, off by default.
- **Close vs end:** closing a tab never ends the session.

Fable's ruling, 2026-11-17:

- **Where the list lives:** the machine page's Overview, frame 59. Its "Sessions on it" section becomes the list of running holders. The Hosts row's "N sessions", frame 28, counts orphans too and opens this list. No new page, and nothing goes in Settings or the Inbox.
- **Each row:** state dot, title, its channel (or the Desk), agent, folder, uptime, the pid in muted text, and "open in a tab" only when it is attached. There is no column for the unattached case.
- **Row actions:** a click opens the session in a tab. The ⋯ menu has Open, then a separator, then a red **End session…**.
- **Orphans:** a second group, "Not in a channel · N", drawn only when there are any. Its rows show agent, folder, uptime and pid, and offer **Adopt…** (a channel or the Desk), Open, then End session….
- **Offline machine:** the page keeps Reconnect as its only action. No End or Adopt is queued.
- **Delete channel:** its sessions end. Frame 71's confirm, which already names the live sessions and says "The agents stop", is the one confirmation. To keep a session, move it to the Desk first. An offline machine ends its sessions when it next links.
- **Remove machine, when reachable:** its sessions end through the session-end path, and the confirm names them. This settles D783's open item.
- **Forget machine, when unreachable:** nothing is ended. The confirm says the machine's sessions keep running there. If it comes back (D776), Bring back resumes them, and Start clean lists them under "Not in a channel".
- **Archive** asks "keep running or end", and keep running is the default. Deleting a single session ends it, because its folder goes to the trash. The delete confirm names the agent and offers moving it to the Desk first.

Owner's amendment, 2026-11-17: "when I close a tab it can ask me 'do u want to end this coding session or do u want to hide it' and then it gives instructions on how to recover them later from a screen that has all of them… So I can close a tab to kill but it has to be intentional." Every terminal tab asks, plain shells included, idle or not; the machine-page list holds every hidden terminal, shells shown as "Shell".

Fable's ruling on the close-tab dialog, 2026-11-17:

- **When:** closing any terminal tab asks, shells included, idle or not.
- **Title:** "Close <agent> on <machine>?", the agent "Claude Code", "Copilot" and so on, or "Shell".
- **Body:** "Hide keeps it running. Open it again from <machine>'s page under Hosts, or find it with Search (⌘K). End session stops it and everything it started; this can't be undone."
- **Buttons, left to right:** Cancel, a red-text **End session**, a blue **Hide**. Return picks Hide, Escape cancels. End session is never the focused button.
- **Second way back:** Search (⌘K) lists every running terminal, hidden ones included. No new menu item or shortcut.
- **"Always hide without asking":** a checkbox in the dialog, which can be turned off in Settings; it only ever applies to Hide. A silent hide shows a toast in frame 32's form, "Hid <agent>. Still running on <machine>.", with **Open** where Undo sits. With the box ticked, End session is reached from the session's ⋯ menu or the machine page. "Always end" is never offered.
- **Layout:** a window-root overlay with the shared scrim, built like frame 75's start confirm: 440 wide, a title, one paragraph, buttons on the right. The red End session follows frame 72's Stop all sessions. Mounted at the window root, never inside the page (D363).

The two-button wording above is superseded by the three-way close below; the trigger (every terminal tab, shells included), the window-root overlay with the shared scrim (D363) and the single remember option stand.

Owner's amendment, 2026-11-17: closing offers three ways. **Keep running** (hide) as above. **Stop, resumable**: the holder and the agent's process group end and the session record stays, exactly as after a reboot; it shows as Stopped on the machine page and in ⌘K, and Resume revives it through the existing revive path (the agent's own resume, same folder, same history). **End for good**: it stops and the session is closed out. The CLI gets `humata sessions stop` and `humata sessions end`, both behind the destroy switch for agents.

Fable's ruling on the three-way close, 2026-11-17: choice 3 is the existing Archive plus an end. History stays readable, the folder follows Archive's rules (frame 65's unpushed-work dialog included); only the running program is lost. A shell has nothing to archive, so it just ends. `humata sessions end` does the same. Stop asks nothing from a menu (it can be resumed); End for good keeps its one confirm. End for good's toast is Archive's own, and its Undo restores the session as Stopped, never as running. The dialog is 520 wide; its red follows frame 72. Machine page ⋯ menu: a running row offers Open, Stop, separator, red End for good…; a stopped row offers Resume, separator, Archive….

Fable's final copy for the close dialog, 2026-11-17 (replaces earlier wording):

- **Title:** "Closing this tab. What should happen to <session title>?" A shell: "…happen to this shell?", or the shell's title if it has one.
- **Muted line under it:** "<agent> on <machine name>", "This Mac" only for this Mac; a shell: "Shell on <machine>".
- **Body, one line per choice, label bold:** **Keep running** "It stays on <machine>. Find it on <machine>'s page under Hosts, or with Search (⌘K)." **Stop** "It ends now. Resume it later from the same place." **End for good** "It ends and is archived. It can't be resumed." (a shell: "It ends. It can't be resumed.")
- **Buttons, left to right:** Cancel, red-text **End for good**, **Stop**, blue **Keep running** (Return). Escape cancels. Neither Stop nor End for good is ever focused.
- **Checkbox:** "Always keep running without asking" (only ever applies to Keep running; can be turned off in Settings).
- **Machine-page menus:** the same labels: Stop, End for good…, Resume.
- **Toasts (frame 32's form):** Keep running, only when the dialog was skipped: "<title> is still running on <machine>." with **Open**. Stop: "Stopped <title> on <machine>." with **Resume**. End for good: "Ended and archived <title> on <machine>." with Archive's **Undo** (restores as Stopped). An untitled shell reads "the shell" where the title goes.
- Search (⌘K) lists every running terminal, hidden ones included, and stopped sessions; no new menu item or shortcut.

Owner's amendment, 2026-11-17: all of D784 works the same on remote machines: each remote machine's page lists its holders, orphans included; Attach, Adopt, the close dialog's three choices, the ⋯ menu, and `humata sessions stop|end` against any machine. Every call is relayed over the peer link to that machine's own daemon, which lists, stops and ends only its own holders. An offline machine keeps Reconnect as its only page action and nothing is queued: Stop and End on an offline machine are refused with a message telling the user to reconnect the machine and try again (CLI and dialog alike). Linux remotes run the same daemon and holder code; their VM-only tests are written now and run in the later Linux VM batch.

Reversal, 2026-11-17 (lane manager agreed): the earlier rule that Archive asks whether to keep the agent running or end it is dropped. Archive is End for good, and the close dialog's Keep running already covers keeping a session alive, so a keep-or-end field on Archive would offer the same choice twice. Every Archive confirm (session menu, close dialog, `humata sessions end`, archive) names the agent it ends.

Shell Stop, settled from the code, 2026-11-17: a start revives every shell tab whose holder is gone (`revive_shell` in daemon/revive), so a shell needs a Stop that survives a restart. Stop writes a `<socket>.stopped` mark next to the holder's socket, then ends the holder by its pid, which keeps its saved output. The start's reconcile skips a marked shell, and `revive_shell` clears the mark when it starts one. MachineService gains StopRunningHolder and ResumeRunningHolder, which Resume serves through the restart doctor's tab repair (a new shell in the folder it was in, with what it showed above). EndRunningHolder ends the shell, then drops its tab, saved output and mark. All three take shell tabs only: an agent tab is refused (FailedPrecondition) with the call to use instead, StopSession, ResumeSession or ArchiveSession. RestoreSession gains `stopped`, which leaves the restored session stopped; it is End for good's Undo.

- **Built so far:**
  - Daemon: ListRunningHolders (session rows, then stopped rows, which are stopped agents and stopped shells, then orphans), StopRunningHolder, ResumeRunningHolder, EndRunningHolder and RestoreSession `stopped`, local and relayed to a box (the relay's machine allowlist). Tests are `running_holders_test`, `archive_test` `undo_of_end_for_good_gives_it_back_stopped_and_resume_revives_it`, and `running_holders_remote_test` (rig).
  - An agent whose holder is gone while the daemon runs shows as Exited (watch.rs `lost`), until Resume or the next start brings it back. Tested by `holder_gone_test`.
  - Teardown: `end_by_pid` ends the program's process groups, plus fixes for return_after_text, kill_matrix and common::kill_holders.
  - Adopt: AdoptRunningHolder (proto, local and relayed) makes an orphan holder a shell-kind session in a channel or the Desk, keeping its pid and socket. The feed announces it and it is watched like a started session. Tested by `adopt_holder_test`.
  - An agent caller of Stop, End or Adopt needs the machine destroy switch (`destroy_holders_test`). Deleting a channel also ends its Launching and Resuming agents (`delete_launching_test`). Removing a reachable machine ends its sessions, and Forget ends nothing (`forget_machine_test`).
  - Clients, CLI and features:
    - Clients (fake and live) and the CLI: `humata session stop|resume|end`, with a session or `--tab`, and `session running`.
    - The close-tab reducer (`features/session/src/close.rs`) and the machine page running list with its row actions, Adopt included (`features/hosts/src/running.rs`).
    - ⌘K terminal entries (`Command::Terminal`).
    - The Archive confirm names the agent and machine.
  - UI views: the close dialog (`ui/src/overlay_close.rs`), the running list (`ui/src/machine_running.rs`) and the End and Adopt confirms (`ui/src/overlay_running.rs`). Their native candidates are in `desktop/tests/native/close_running.rs`, which is kept out of `native_test` until it passes.
  - Green runs:
    - daemon: 137, and 139 for remote;
    - clients, features and CLI: 136, 138, 140 and 147;
    - ⌘K (frame 30): `terminals_test` red in 136 and 149, green in 151 and 157. A terminal is searched on its title and machine, its hint is the machine, and it sits below its session;
    - the tab X (Fable's ruling): design `session::tab` always draws the X. `design_test` green in 154, `native_test` green in 165 after recording rounds 150–164. Every recorded diff is the tab row plus `Button "Close tab"`, except frame 30 (the new order) and frame 65 (Archive names the agent and machine).
    - frame 28: the Hosts row's "N sessions" counts the machine's running orphans (`features/hosts`, read when a machine comes online; a failed read keeps the last count). `hosts_orphans_test` red in 170 (no `Holders` or `orphans`), green in 172. Opening Hosts reads them again (native red in 176, green in 178);
    - the close dialog and running-list native cases are back in `native_test` with the toast cases: green in 174. Their checks find the dialogs' root groups, since a card inside its focus trap can't be found by its own id. The overlay now redraws when the close dialog opens (`ui/src/bridge.rs`), not seen red;
  - Open:
    - a session mid-Resuming isn't ended when its machine is removed (done 2026-11-18; one left Resuming with no restart that named it stuck still answers busy, and Remove names it);
    - an adopted holder is always a shell-kind session: settled by Fable's B2 ruling below, being built for 0.2.5;
    - opening an orphan from the machine page needs an attach-by-tab call;
    - orphans aren't in ⌘K;
    - there is no `humata sessions adopt`;
    - the CLI sends no machine when `--machine` is left out;
    - an extra agent tab's close offers only Keep running: settled by Fable's B1 ruling below, being built for 0.2.5;
    - the VM-only tests aren't queued.
  - The rest is listed on the GAPS_REVIEW D784 line.

Fable's rulings on the D784 candidates, 2026-11-18 (for 0.2.5):

- **Accepted as drawn:** A1 and A5.
- **A2, untitled shell:** an untitled shell session reads "Shell" in its sidebar row and its header, never an empty title.
- **A3, Stop toast:** a stopped session uses frame 73's grey stopped square and a muted "Stopped", with a plain Resume button, not the red ✕ "Agent exited". The Stop toast drops the archive glyph.
- **A4, keep-running toast:** shows the session's own state dot, not the archive glyph.
- **A6, frame 59:** keeps its fixture and its right column, and shows "Not in a channel · 2". Every row stays inside its card; session rows may drop the folder column. No menu is open in the frame. The row ⋯ menu is its own new design frame, painted at the window root, with the red item "End for good…".
- **A7, End confirm:** reads "End this shell for good?", with the folder, machine and pid in the body.
- **A8, frame 28:** identical to the golden except "8 sessions", with devbox-1 still selected.
- **B1, an extra agent tab's close:** (b) is the rule: an extra tab running an agent gets its own per-tab agent resume, so its close offers the same three ways as the session's own tab. Until that resume is built, (a) holds: the close offers Cancel, End for good and Keep running, with no Stop, since a stopped extra agent tab could not be brought back in its conversation. End for good ends that tab's holder and its process group; the session and its other tabs are untouched.
- **B2, an adopted holder:** (b) is the rule: an adopted holder keeps the agent kind its holder reports (the program its holder was launched with), never guessed from process names, so an adopted Copilot is a Copilot session, not a shell. Its conversation isn't known to Humata, so it offers Stop only when its conversation is known; otherwise Keep running and End for good are its ways out.
- **A stopped session's status (main's ruling, 2026-11-18):** StopSession leaves a session at the new `SESSION_STATUS_STOPPED = 13` ("the person stopped this session's agent; Resume brings it back"). Before, the daemon mapped `AgentState::Stopped` to `EXITED`, which drew the red ✕ "Agent exited". `MACHINE_STOPPED` was not reused: it means the whole machine's sessions were stopped (D264) and its fix is Start sessions, not Resume. The client maps it to its own `Status::AgentStopped`, which draws frame 73's grey square and a muted "Stopped" with a plain Resume, in the sidebar, the header, ⌘K and the CLI. Places that asked "has the agent ended?" now use `Status::agent_ended` (Exited or AgentStopped). Older clients read it as UNSPECIFIED (api/GRPC.md §11).
- **B2 as built (2026-11-18):** Adopt saves the reported agent kind with an empty conversation. StopSession refuses an agent whose conversation is unknown, unless its kind is a plain shell (`AgentKind::is_shell`, so any shell kind passes, not only zsh). Revive gives `ResumeFailure::ConversationUnknown` (`REASON_CONVERSATION_UNKNOWN = 6`), whose fix is End for good, or keep it running. The client doesn't yet hide Stop for such a session; the daemon's refusal says what to do.
  - Fixed (VERIFY 0.2.3, 2026-11-18): a resumed agent's own tools were refused as IDENTITY_MISUSED and its token revoked. A resume (Resume, Restore, revival after a restart, a move) saved the tab's new holder pid only once the resume was verified, and the agent's MCP child called in before that, while the tab still named the old holder. `daemon/revive` `resume` now points the tab at the new holder as soon as it is spawned (`hand_tab_to`); the process-tree check stays as strict as before.
  - Done (CLI, 2026-11-18): `humata attach --tab t [--machine m]` with no session reaches a terminal no channel holds; `humata session adopt <tab> [--machine m] [--channel c]` calls AdoptRunningHolder and prints the session; `humata session restore --stopped` is End for good's undo; tab and `session running` commands name this machine when `--machine` is omitted. Also `humata shell --session s` (OpenShellTab) and `humata agents edit <agent> --asks-in-inbox true|false` (SetAgentEnabled). Tests: `cli:running_holders_test`, `shell_session_test`, `agent_switches_test`. The GAPS_REVIEW items "no `humata sessions adopt`" and "opening an orphan needs an attach-by-tab call" are done (that file is outside this lane).
  - Done (failed resumes, 2026-11-18; VERIFY 0.2.3 e2e023 c2): a resume or restore that fails ends the holder it started, on this machine and on a box. Never auto-killing sessions doesn't cover this: that holder is the failed op's own debris, not a session anyone kept. Revive already ended it on every failure after the holder is up (the agent exits, verify errors, seeding, saving the tab). What the owner saw was a restore still checking its agent: until the check passes, the agent has no tab, so ListRunningHolders showed the holder with no session and saved a session-less shell tab for it. `running_holders.rs` `listed` now finds such a holder by its agent's socket, lists it as that session's, and saves no tab for it. It only ever ends the holder this op started, never one that was already running. Tests: `resume_exits_at_once_test` (resume; a restore whose agent exits after 4 s) and `resume_exits_at_once_remote_test` (box-1).
  - Done (daemon, 2026-11-18): Resume of a session that was ended for good is refused with FailedPrecondition, and no agent is started. Before, it was accepted: VERIFY 0.2.3 e1 F1 on This Mac and e3 on humata2-linux. `SessionSvc::refuse_if_archived` (`session_service/archive.rs`, `ArchiveError::Ended`) runs in `resume` right after the closed-Desk check, before the gate and before the remote hand-off. So a resume relayed to a remote session is refused on the host, which records remote archives in its lineage. The fix text points to Undo or `humata session restore`, which brings the session back stopped. The fake and `SessionAction::Resume` refuse it too, and the app offers no Resume on an archived row.
  - Done (daemon, 2026-11-18): Forget machine ends nothing, and the machine's holders become unowned there. Before, the machine still recorded the forgotten host's sessions, so their holders stayed listed in their old channel and neither End nor `end --tab` would take them (VERIFY 0.2.3 e3). Now the host sends `MachineService.LetGoOfHost` (new; relayed host-only) during the forget if the machine answers, and otherwise at the machine's next link (`follow_let_go`). The machine drops those session records and the channels left without sessions, and keeps their tabs as shells with no session. Each holder is then listed under Not in a channel and can be attached, adopted or ended. Remove machine sends the same after ending its sessions. Tests: `forget_machine_test`, which covers the online and offline cases and Adopt (red run 175, green run 177).
  - Done (daemon, 2026-11-18): Remove machine ends a session that is mid-resume there. Before, the machine refused the stop as busy, and the removal archived the session while its resume left a holder running there. The host now asks the stop again until the resume settles, within the 30 s deadline. Any session still not ended is logged and shown as a FAILED `end-sessions` step on the Remove operation, naming the session and machine and how to end the holder. Stop also ends the old holder of an agent left Resuming by a run that ended mid-resume (`left_resuming`). One that a live operation is bringing up stays busy. This settles the open item "a session mid-Resuming isn't ended when its machine is removed". Test: `forget_machine_test` `removing_a_machine_mid_resume_ends_the_resumed_session_there` (red run 184, green run 186).
  - Fixed (0.3.0 regression, 2026-11-18): a move from a remote machine of a session whose agent had written no conversation yet (a Copilot or Codex session with no first message) failed at "Starting the agent on the new machine". The move's hand-over stops the old agent through that machine's own StopSession, and 0.3.0's refusal of a Stop with nothing to resume refused it. The refusal is for the person's Stop, which would leave a session that can only fail to resume. A move doesn't leave it stopped: the new machine starts it fresh (D757's after-move resume). So `StopSessionRequest.for_move` (new field 3) marks the move's own stop. The machine honours it only from the host (`Caller::User(Host)`) on a daemon that isn't the host, the same trust as D757's `MODE_AFTER_MOVE`: it looks once for a conversation the CLI picked itself (D670), doesn't wait the 2 s grace, and stops the agent. Anyone else's `for_move`, and every Stop the host relays for the person (`stop_remote`), is the plain Stop and still refused. A move from this Mac never went through StopSession, so it wasn't affected. Remove machine and the channel Desk watch still send the plain Stop, so a remote agent with no conversation yet is still refused there: Remove names it in its failed `end-sessions` step with how to end the holder (open: whether Remove should end such an agent outright). Tests: `move_session_test` (the three no-conversation moves), `running_holders_test` `an_agent_with_no_conversation_written_yet_is_not_stopped` (a local Stop asking `for_move` is still refused).

Channel delete on other machines, 2026-11-18 (VERIFY 0.2.3 finding; main's scope for 0.2.4): PlanDeleteChannel offered a plan for a channel that a remote machine holds. The plan listed none of the remote live sessions, called the online machine offline, and DeleteChannel then refused it as not built. Now, while the relayed remote delete isn't built, a channel that another machine holds, or that has live sessions on another machine, gets no plan. PlanDeleteChannel and DeleteChannel both refuse it up front with FailedPrecondition (`refuse_held_elsewhere` in `channel_service/delete.rs`). The proto has no "not appliable" field, so a refusal is used. The refusal names each machine as online or offline from the host's links, and each live session there with its last-reported work (`remote_sessions_of`). Its fix: move each session to keep to the Desk, then remove each machine from the channel, reconnecting an offline one first, and plan again. A plan now always has `offline_machines` empty. The D784 semantics stay: deleting ends the channel's sessions behind a confirm, and moving a session to the Desk keeps it. The full relayed delete is still open (GAPS_REVIEW).

## D785: A screen question whose choices the screen cuts off is raised with no choice, only its terminal (VERIFY 0.2.3 finding with Copilot, main's ruling, 2026-11-18; amends D767, uses D781). Decided, built. Superseded by D845 (INBOXHOOKS)

- **Finding:** a headless Copilot session (no writer attached, pty 80x24) draws its folder trust box too short for its list: only `1. Yes` shows, above a footer saying `pgup/pgdn to scroll`. The card was raised with the one choice Yes, so the Inbox offered trust with no way to refuse; it got all three only once someone attached and resized.
- **Decided:** the card never offers part of a list as the whole. When the list is cut off, the card names the question, offers no choice and says to open the terminal to answer. With no choices and only choices taken, every reply is refused and nothing is typed; Dismiss and the terminal under the card (D781) stay. The UI needs no change: an on-screen card with no choices already shows only Dismiss and the terminal.
- **Cut off when any of:** the list starts past `1.`; a row below it says the list scrolls (`to scroll`); or the choices lack an answer the question always has. For folder trust that means at least two choices, one of them No. The last check doesn't depend on the agent's footer wording, and it also covers a box still drawing (`1. Yes` alone, or two choices without the No).
- **Not chosen yet:** reading every choice anyway. A taller virtual read of the kept output can't help, because Copilot drew the box clipped. Typing pgdn is a key the person didn't send. Resizing the holder's pty while no writer is attached would work, but it changes the sizing contract (D14: only attaches that can type resize). It stays open for a follow-up. Chosen in D786 (Fable, 2026-11-18): the holder starts at 80x50, plus a one-time grow while no writer is attached. It goes in the release after 0.2.4; 0.2.4 ships this fallback.
- **Code:** `screen_ask.rs` (`Asked.whole`, `asked_of`, `answers_complete`, `Asked::offered`, `card_text`), `inbox/permission.rs` (`decide` refuses a reply to a card with no choices, with its own message).

## D786: A holder starts 80x50, and a question box cut off by a short screen grows the screen to 50 rows once, only while no one who can type is attached (Fable 5.2 ruling, 2026-11-18; closes D785's "Not chosen yet"; amends D14 and D271 point 3; uses D767, D781, D782, D393). Decided (by Fable), next release after 0.2.4. Pick 2 superseded by D845 (INBOXHOOKS); Pick 1 stands

- **Problem:** D785 never offers part of a list. So a headless session's folder trust prompt reaches the Inbox with no choice (on an 80x24 pty, Copilot draws only `1. Yes`). The person must open the terminal for every such question. "Remember this folder" is never offered, so trust is asked again on every start and resume.
- **Pick 1, the starting size:** every holder humatad starts is 80 columns by 50 rows, for agent and shell tabs alike. One shared constant replaces `COLS`/`ROWS` in `session_service.rs`, `desk_service/start.rs`, `desk_service/channel_desk.rs` and `running_holders/adopt.rs`; the holder binary's own default stays 80x24. This is not a resize: the program is born at that size and gets no SIGWINCH.
- **Pick 2, the grow:** when a look (`screen_ask`) reads a question whose list the screen's height cuts off, humatad resizes that tab once, to its current columns by 50 rows. It does so only if all of these hold:
  1. The cut-off is a height signal: the list starts past `1.`, or a row says it scrolls (`whole == false`). A list that only lacks an answer (`answers_complete`, possibly still drawing) never causes a grow.
  2. The tab has no attach that can type (`Writers` count 0 on the daemon where the session runs, relayed attaches included). Read-only attaches don't block it; they hear Resized as for any resize.
  3. The pty has fewer than 50 rows. So it only ever grows: it never shrinks, never changes columns, and can't repeat until a writer has made the screen short again.
  4. The agent's dialogs come to the Inbox (D782's switch on).
  The grow goes through `resize_tab`'s order (daemon screen, then holder) and marks the repaint as not new output (D393).
- **The card:** a cut-off read that causes a grow raises no card. The next look raises the card, and it must come within 3 s of the grow whether or not the program writes. That card carries every choice, or it is D785's no-choice card if the list is still cut off. The session is NEEDS_INPUT from the first read. A cut-off read that doesn't qualify for a grow raises D785's card as today.
- **Attach and detach:** unchanged. A writer's attach sizes the program to its own size (D14, D750); the Inbox terminal never sizes it (D781). Nothing reverts on detach. Humata never shrinks a pty.
- **Amends D14 / D271 point 3:** resizes come only from attaches that can type, with one exception: humatad itself may grow a tab's rows under Pick 2.
- **Never:**
  - typing pgdn or any other key to scroll the box;
  - resizing while a writer is attached;
  - resizing for a box that is merely still drawing;
  - resizing on a timer or a retry.
- **Fallback:** D785 stands for everything Pick 2 doesn't fix:
  - a writer attached with a short pane;
  - a program that doesn't redraw;
  - a box that needs more than 50 rows.
- **Owed live check:** on a fresh headless session at 80x50, local and remote, Copilot draws all three trust choices, and "Yes, and remember this folder" picked from the Inbox stops the box on the next start. If 50 rows isn't enough, raise the constant; the rule doesn't change.
- **Tests (screen_questions_test, with a test agent that clips below N rows and redraws on SIGWINCH):**
  - a headless trust box offers every choice;
  - a short screen with no writer grows to 80x50, and exactly one card is raised, with all choices;
  - with a writer attached there is no resize, and D785's card is raised;
  - a reader doesn't block the grow and hears Resized;
  - a box still drawing is never resized;
  - a box still clipped at 50 gives one resize, then D785's card;
  - columns are kept and the pty never shrinks;
  - the grow types nothing;
  - with the switch off there is no resize;
  - a program that ignores the resize still gets its card within 3 s;
  - the grow is not unread output;
  - the remote rig variant passes, with a relayed writer blocking the grow.

  Unit tests cover the pure grow decision. The writer-attach sizing test and D781's no-resize test stay unchanged.
- **Upgrade:** none. Holders already running at 80x24 are grown by Pick 2 at their next cut-off question.
- **Reverse:** set the constant back to 24 rows and drop the grow call in `screen_ask`.
- **Fable's answers (2026-11-18, through the coordinator):**
  - The shells revive and recover respawn start at the same size: `revive/src/reconcile.rs` (restart of a shell tab) and `sessions/src/recover.rs` (a start cut short, finished on restart) use `START_COLS`/`START_ROWS`, which `humata2_revive` defines once and humatad's `holders.rs` re-exports.
  - The card-less hold a grow leaves while the screen redraws is not a card shown: `look` treats an `Open` with `grew` as nothing shown, so the next look raises the card whatever the screen reads.
- Done (daemon, 2026-11-18): every holder humatad, revive and recover start is 80x50. `screen_ask::look` reads the screen with its size (`holders::screen_and_size_at`). For a list cut off by height (`Asked.cut_by_height`), with `Writers::count` 0, fewer than 50 rows and D782's switch on (`grows`), it keeps a card-less hold (`Open.grew`) and grows the tab through `attach::grow_rows` (`resize_tab`'s order, `read_marks::tab_resized`). It then looks again on the program's redraw or 1 s later (`LOOK_AFTER_GROW`). A grow is not tried twice for one hold, even when it fails. Tests: 11 D786 cases in `screen_questions_test`, `screen_questions_remote_test` (rig: a relayed writer blocks the grow), and `grows` / `cut_by_height` unit tests in `humatad2_unit_test`. Red run 187: the 7 grow cases and the remote case failed because the agent never drew at 50 rows. The guard cases passed, and the headless case was already green from Pick 1. Green: run 189 (both D786 targets, `humatad2_unit_test`, `terminal_test`, `terminal_program_test`, `checkpoint_test`, `read_marks_test`, `running_holders_test`, `adopt_holder_test`, `desk_test`, `sessions_test`, `revive_unit_test`, `sessions_unit_test`).

## D787: Fast mode is an app setting, off by default, and with it on terminal cursors stop blinking, so an idle terminal draws nothing (FAST lane, 2026-11-18; uses D420's setting pattern, D770). Decided, built

- **Setting:** Settings › General › Fast mode, a switch saved with the app's other preferences (`Preferences.fast_mode`, as option-as-Alt and free-hidden-terminals are). Its compiled-in default is off. `HUMATA2_FAST_MODE` (`1`/`true`/`on`, or `0`/`false`/`off`) sets the default, so a test run can have it on (`--test_env=HUMATA2_FAST_MODE=1`) or off; a saved choice wins over it. The palette finds the row.
- **Not chosen yet:** keeping it in `~/.humata2/config/settings.toml` with the daemon's portable sections. Only the app reads it so far, and the app reads no daemon settings at launch; moving it there needs a SettingsService field, a client call and a launch read. Reversible: add `fast_mode` to the portable sections when a daemon or CLI behaviour joins fast mode.
- **Trade-off 1, cursor blink:** a focused terminal whose program asked for a blinking cursor (most shells and agents do) flips the cursor every 600 ms, and each flip is a frame. In fast mode the cursor stays solid, and an idle terminal asks for no frame at all. The view already had the switch (`TerminalView::set_cursor_blink`); the app now sets it for every session terminal and the Inbox terminal.
- **Measured** (`bazel run -c opt //apps/humata2/terminal/view:frame_bench`, 200x60 terminal, one idle minute of the test clock, focused, blinking cursor asked): blink on, 100 frames and 186.7 ms of drawing a minute; fast mode, 0 frames and 0 ms. One frame of this terminal costs 1.8 ms whether or not a row changed (idle median 1.80 ms, one row 1.89 ms), so each frame saved is the whole frame's cost.
- **With fast mode off:** nothing changes; the view's blink was already on by default and setting it again is free.
- **Tests:** `features/settings:fast_mode_test` (default off unless the variable turns it on; toggles; in General and found by the palette); `terminal/view:view_test` `with_blinking_off_no_blink_is_scheduled` covers the view.
- **Reverse:** drop the `set_cursor_blink` calls in `ui/src/terminals.rs`; the setting can stay with no effect, or go with its row.

## D788: A terminal cursor blinks only while its window is active, so Humata in the background draws nothing for its cursors; applies always, fast mode or not (FAST lane, 2026-11-18; uses D787). Decided, built

- **Finding:** `TerminalView` blinked whenever its view had focus in its window. A window keeps its focused view when another app comes to the front, so a Humata window behind the editor or browser still drew a frame every 600 ms for each focused terminal whose program asked for a blinking cursor, all day.
- **Decided:** the blink runs only while the window is active, as in Ghostty. In the background the cursor stays drawn, steady, in its focused style; nothing else about the cursor changes. GPUI draws the window again when it becomes active, and that frame starts the blink over. The blink is decided before the frame is built, so a cursor stopped in its hidden half is drawn shown.
- **Plain win, no setting:** the only visible difference is a steady cursor in a window the person isn't looking at, which is what other macOS terminals show. It applies with fast mode off too.
- **Measured** (`bazel run -c opt //apps/humata2/terminal/view:frame_bench`, 200x60, one idle minute of the test clock, focused, blinking cursor asked, window in the background): before, 100 frames and 186.7 ms of drawing a minute (the old code didn't look at the window, so this is the active-window case of the same run); after, 0 frames and 0 ms. Per-frame cost is unchanged (idle frame median 1.80 ms before and after).
- **Tests:** `terminal/view:view_test` and `view_fast_test` `a_window_in_the_background_stops_the_blink` (written first; not run red, but the old `render` passed `is_focused` alone, which a deactivated window keeps, so the blink stayed scheduled).
- **Reverse:** pass `focused` alone to `schedule_blink` in `TerminalView::render`.

## D789: A holder's kept output never takes more memory than its scrollback limit, and rewriting its saved log copies nothing; applies always (FAST lane, 2026-11-18). Decided, built

- **Finding:** a holder whose program printed more than its scrollback (8 MiB, humatad2's default) held 42.1 MiB resident: `Scrollback::push` appended before trimming, so its `VecDeque` grew past the limit and doubled to 16 MiB, and each rewrite of the saved log built the whole scrollback twice in memory (`since(0)`'s copy, then one `Vec` of every record) before writing it.
- **Decided:** `push` makes room first and grows its buffer by doubling only up to the limit, keeping just the tail of a piece longer than the limit. `rewrite` writes records in 1 MiB batches straight from the kept bytes (`Scrollback::each_piece`). Nothing kept changes: the same newest bytes at the same offsets, the same saved-log format.
- **Plain win, no setting.**
- **Measured** (`daemon/holder:cost_bench`, new: a real `humata2_holder`, no client attached, 200x60; fastbuild runs 192 and 194): after 24 MiB of output then idle, 42.1 MiB resident before, 19.2 MiB after (18.3 MiB on -c opt); a holder whose program printed nothing, 7.7 MiB either way; idle CPU 0 ms over 10 s either way. `holder_unit_test` `a_full_scrollback_holds_no_more_memory_than_its_limit`: 16,777,216 bytes held for an 8,388,608-byte limit before, at most the limit after.
- **Also measured, nothing to fix:** an idle holder and humatad2 on a quiet peer link run no timers (holder sync runs only after writes; `peer_link_latency_bench` -c opt: humatad2 idle on an open link 11.9 MiB, 0 ms CPU over 10 s; WhoAmI round trip p50 45 µs, p99 80 µs).
- **Tests:** `daemon/holder:holder_unit_test` scrollback tests (red first in run 193), `holder:all` (power_cut, terminal, end, shutdown, keyed_input).
- **Reverse:** restore the push-then-trim loop in `scrollback.rs` and the whole-`Vec` `rewrite` in `saved.rs`.

## D790: Humata ships six skills for plain asks, as thin pointers, and its daemon installs and updates them in each agent's skills folder (owner's ask, Fable 5.2's picks, 2026-11-18; closes flow case board/09 step 4; uses D745, D748, D743). Decided (by Fable), built

- **Asked by the owner:** skills Humata installs and updates by itself, thin pointers to Markdown files that hold everything, so an agent can act on a plain ask about the board, the remote machines and the rest.
- **Found:** nothing installed Humata's own skills. `skills/` held `restart-doctor`, `order-humata-review` and `render-humata-review`, and no code read the folder. The skills sync (SK-1, `skills_service.rs`, `peer_skills.rs`) copies the *person's* skills from the host's `~/.claude/skills`, `~/.codex/skills` and `~/.copilot/skills` to machines and links them there (D745). So no agent, on any machine, ever saw a Humata skill.
- **The list, by value** (`skills/README.md` has purposes and trigger phrases):
  1. `humata-board`: items, statuses, assignees, depends-on edges, a plan as items.
  2. `humata-sessions`: sessions and channels; start, fork, continue, talk to another session, history, end.
  3. `humata-remote`: machines; start, fork and move sessions there, files between machines, transfers, skills sync, scripts, provisioning.
  4. `humata-inbox`: the owner's approval; check a rule, a pending request, ask ahead, ask a question.
  5. `humata-pull-requests`: push and open pull requests, `stack`, PR review drafts.
  6. `humata-files`: `shared/`, the channel folder, main clones, other sessions' files, the trash.
  7. `restart-doctor`, `render-humata-review`, `order-humata-review`: kept, made thin (their text moved to `guide.md` unchanged).
- **Merged or dropped:** channels into sessions; transfers into remote; stack into pull requests (it points at `stack guide`, the tool's own manual); Desk is one command in sessions; settings, agent kinds, access rules and pins are the owner's alone and get no skill; no general "humata" skill, because `shared/basics.md` says what it would and descriptions cost context in every session.
- **Thin pointers:** a `SKILL.md` is front matter (`name`, and a `description` that carries the trigger phrases) and a few lines naming the files to read. The reference files sit in the skill's folder: commands with flags checked against `cli/src/args.rs`, workflows, what needs the owner (SPEC IN-17), failures by exit code, and "Not in the CLI". `skills/shared/basics.md` (who you are, flags, outcomes, exit codes, pending approvals) is copied into every installed skill as `shared/basics.md`, so a skill folder stands alone when it is copied, synced or contained.
- **Install (the pick):** the skills tree is packed at build time (`//apps/humata2/skills:bundle`, a glob, so a new folder needs no listing) and embedded in `humata2_machines`. Each time `humatad2` starts (`serve.rs`, host and machine alike) it writes every skill into `~/.claude/skills`, `~/.codex/skills` and `~/.copilot/skills` (`peer_skills::AGENT_FOLDERS`) for each agent whose own folder (`~/.claude`, `~/.codex`, `~/.copilot`) exists.
  - Each installed folder holds a `.humata-skill` mark. A marked folder is replaced (written beside, then renamed in) when it differs from what the daemon carries, and removed when the daemon no longer ships that name. So a release updates the skills at the daemon's next start, and a renamed skill doesn't leave its old copy behind.
  - A folder, file or link of the same name without the mark is the person's and is never touched; the daemon's log says so. Deleting the mark makes an installed copy the person's.
  - A failure (a folder that can't be written) is logged with its fix and the daemon starts anyway.
- **Why the agents' own folders, when D748 writes nothing into an agent's config:** a skills folder is where each CLI looks for personal skills, D745 already links there on machines, and Codex has no flag for an extra skills folder. One mechanism covers all three kinds, contained launches (they copy the person's skills at each launch) and agents the person runs outside Humata. Considered: Copilot's and Claude Code's `--plugin-dir` with a `skills/` folder in the state folder. It would write nothing in the home, but leaves Codex out and names the skills `plugin:skill`.
- **Never a third-party install:** the daemon makes `skills/` under an agent's folder but never the agent's folder, and installs no CLI. The Settings › Agents install switch (D720) is not involved.
- **Sync:** the scan (`humata2_machines::scan_skills`) leaves marked folders out and doesn't count them as excluded. Each machine's daemon installs the copy that matches its own `humata`, so nothing waits for a first sync to be approved (SK-5) and nothing clashes.
- **Tests never reach the real home:** the home is `--skills-home` when given; otherwise `$HOME` only when the daemon serves from its default state folder. A daemon given `--state` and no `--skills-home` installs nothing.
- **Gaps left:**
  - CLI actions the skills needed and the CLI lacks (each is listed in the skill as "Not in the CLI", with the MCP tool that does it): print one board item; a reason on a board change; prepare a view; ask the owner a question (`inbox_ask`); wait on or withdraw a request; a pull request's title, body and base on `session push`; review drafts, notes and comment replies; open a network forward; read part of a large file.
  - OpenCode and agents-file kinds get no skills: `AGENT_FOLDERS` has only the three. Adding a kind is one row there.
  - An agent whose home is elsewhere (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `COPILOT_HOME` set in the person's shell) is not found; D193 keeps home variables out.
  - An agent CLI installed while the daemon runs gets the skills at the daemon's next start, not at once.
  - A running agent reads a skill's new text when it next loads the skill; Claude Code and Codex list skills at session start, so a new skill shows in sessions started after the daemon's restart.
  - Skills are Markdown only: the installer sets no executable bit.
  - `api/CLI.md` §5.3 says `humata diff --against base|last-turn|turn n`; `args.rs` takes `main`, `uncommitted`, `last-reviewed`, `fork`, `pr` or a session. The skills follow `args.rs`.
  - TP-17's delegation rule is one paragraph in `shared/basics.md`; the MCP tool descriptions still don't carry it.
- **Tests:** `daemon/machines:machines_unit_test` (`own_skills` tests: install, update, unchanged, the person's own kept, the mark deleted, retired, shared files, the packed format, and that every shipped skill is named as its folder and holds every file its `SKILL.md` points to); `daemon/humatad:skills_test` `the_daemon_installs_its_own_skills_for_the_agents_set_up_and_never_syncs_them`.
- **Reverse:** drop the `install_own` call in `serve.rs`; delete `own_skills.rs`, its `lib.rs` lines, the mark check in `skills.rs` `walk`, and the bundle from `machines/BUILD.bazel`. Installed copies stay until deleted by hand (each holds the mark).

## D791: Showing a change and reviewing it are two skills: render prepares a navigable stack and never judges; a new skill reviews when asked (owner, 2026-11-18; narrows D59; amends RV-33 and PR-22; adds RV-36; uses D790). Decided (by owner; the split of the skills by Fable)

- **Asked by the owner:** "Render review i want it to just show me the things that have changed. If its a stack of changes I want to see it as a stack. I want to see it across the entire stack for all repos involved. I want another one to actually review and put inline stuff. That distinction is key because sometimes i just wanna look at the code." And: "Render tells the agent to prepare the stack for review like it will send the stuff so i can logically navigate a stack of changes across a 1 to many repos."
- **Three skills, three asks, no overlap:**
  - `render-humata-review` ("show me", "what changed", "show the stack", "just look"): prepares the change as a stack of layers across every repo it touches and sends it to the view. No notes on lines, no opinion. A layer's one-line purpose is navigation.
  - `order-humata-review` ("walk me through it", "explain this change"): a reading order, groups and neutral notes that explain. No verdict.
  - `review-humata-change` ("review this", "find problems", "leave comments"): reads the whole stack, puts each finding on its lines with `review_note`, drafts PR comments with `review_draft` only when the change is a pull request the owner is reviewing, and ends with a short verdict. It never posts.
  Each description names the other two as "not for", so "show me" never triggers a review and "review this" never just renders.
- **D59, narrowed:** the review chat's agent still doesn't review on its own initiative, and a review session's first message still isn't "review this". When the owner asks for a review, the skill reviews: findings and a verdict are then what was asked for. PR-22's "gives no verdict of its own" reads "unless the user asks for one". Posting stays the owner's (PR-8, PR-12).
- **How the agent picks layers** (`skills/shared/stacks.md`): (1) a stack that exists, `stack` topics (`stack status --json`) or chained pull requests; (2) otherwise one layer per commit; (3) otherwise a logical split of uncommitted work, and it says it chose the split. Layers with the same topic or branch in two repos are one layer.
- **What is sent today:** the view has no layers, and a diff spec always ends at the checkout, so one layer's own diff can't be shown unless it is the top one. Until STACKVIEW, render sends one `review_prepare`: the whole stack as one spec per repo, the files ordered layer by layer, and one group per layer (title "1. codec", the purpose as its note). A file two layers change appears once, under the lower layer. The skill tells the owner this and that the stepped view is being built.
- **The code gap is a milestone:** `workflow/milestones/STACKVIEW.md`. `review_prepare` takes a `stack` (ordered layers; per repo a `base..head` range or a set of worktree paths; optional pull request and one-line purpose); the view lists layers and shows the whole stack or one layer; `review_focus` steps layers; notes can sit on a layer. RV-36 is its rule.
- **Shared reference files:** the diff spec fields moved from render's guide to `skills/shared/diff-spec.md`, and the stack rules are `skills/shared/stacks.md`, because render, review and order all need them and shared files are copied into every installed skill (D790).
- **Not verified:** whether `review_note` is accepted outside a PR review session. The skill tries it and, if refused, sends its findings as the `notes` of one `review_prepare`, which works in any session. STACKVIEW's lane should settle it.
- **Reverse:** delete `skills/review-humata-change`, restore render's and order's descriptions, and drop RV-36 and the PR-22 amendment.

## D794: Mermaid diagrams zoom and pan in place and open large in the view, and are drawn again sharp once a zoom rests (owner, 2026-11-18). Decided (by the brief), built

- **Asked by the owner:** "I cant zoom in and navigate around mermaid diagrams".
- **In place:** pinch, or ⌘-scroll (Ctrl-scroll elsewhere), zooms about the pointer. A drag pans while zoomed and selects no text. A plain scroll still scrolls the document, so a diagram never traps the page. The frame keeps its size: zooming never measures the document again, it only repaints (`cx.notify`, not `invalidate_inline_layout`).
- **Controls:** shown only while the pointer is over the diagram or it has focus: zoom out, zoom in, Fit, Open. `+` (or `=`), `-` and `0` zoom, zoom out and fit a focused diagram. A click focuses it.
- **Open:** Open, or a double-click, shows the diagram in place of the document inside the same view, with a toolbar (name, zoom %, zoom out, zoom in, Fit, 100%, Close). It zooms the same way, and a plain scroll pans there. Escape or Close returns and focuses the diagram again. It is an overlay inside `MarkdownView`, not an app tab: the crate stays generic and no app wiring is needed. Moving it into a real tab is the app's call later, when it has a diagram tab kind.
- **Sharpness and memory:** each diagram keeps its laid-out SVG next to its picture. Once a zoom has rested for 150 ms, the diagram is drawn again at the smallest scale bucket (2, 4, 8 or 16 times natural) that is sharp at that zoom and the screen's scale. The nearest sharper picture already drawn stands in until then. The cache is keyed by source, theme and bucket. Zoomed pictures are capped at 4 and 24M pixels in all, oldest first; base pictures are never dropped for them. One picture is capped at 8192 px a side and 12M pixels, so a very deep zoom into a large diagram gets slightly soft rather than using more memory. Every picture the cache drops is freed on the GPU (`drop_image`).
- **Keyed by the block's first byte:** a zoom resets when text above the diagram changes. The opened diagram closes if its block goes.
- **Code:** `markdown/src/zoom.rs` (pure viewport math, key mapping and bucket choice), `markdown/src/view/diagram_zoom.rs` (state, handlers, controls, Open), and `markdown/src/mermaid.rs` (`Drawn`, `raster`, bucketed keys, bounded zoomed pictures).
- **Tests:** `markdown:markdown_test` and `:markdown_fast_test` (`zoom::tests`: fit, zoom about the pointer, limits, pan edges, bad input, keys, buckets; `mermaid::tests`: a zoomed picture is kept beside the base, only a few zoomed pictures are kept). `markdown/headless:rendering` (`a_zoomed_diagram_is_drawn_again_sharp_once_it_rests_and_a_plain_scroll_never_zooms`, `a_double_clicked_diagram_opens_large_and_escape_brings_the_document_back`).
- **Reverse:** delete `zoom.rs` and `view/diagram_zoom.rs`. Restore `render_diagram`'s plain `img`, the two-part cache key, and `render` returning a picture. Narrow the BUILD glob back to `src/*.rs`.

## D793: The sidebar's Desk lists the Desk sessions you used last on every page, up to a number set in Settings, and folds like a channel (owner, 2026-11-18; supersedes D148's sidebar bullet and D100's "only on the Desk"; amends DK-1). Decided (by Fable), built

- **Asked by the owner:** "I want the top 3-5 (configurable) desk sessions to appear if I expand it to appear on the side always."
- **The Desk row folds.** While there is at least one open Desk session, the Desk row has a trailing chevron in its count slot: ChevronDown open, ChevronRight folded, small, tertiary (selection text when the row is selected). The chevron toggles the fold. A click on the row still opens the Desk page. With no open Desk session there is no chevron and the row is a plain link, as before.
- **Keyboard**, only while the row has a chevron: the Desk row is a stop. ← on it folds, → unfolds or goes to its first row, Enter opens the Desk page. ← on a row under it goes to the Desk row.
- **Fold:** open by default, kept between launches as `Folding.desk` (true is folded; `#[serde(default)]` so older saved folding reads as open). Expand all opens it and Collapse all folds it.
- **Open, on every page** (the Desk page included), the same rows: at most N Desk sessions. The session the main area shows always takes one of the N. Membership is that session, then the most recently used (`last_used.order` descending, ties by id). The rows are drawn in the order the sessions were started, so a row doesn't move when used; the Desk page's Sort doesn't apply. When some are left out, a last row "K more" opens the Desk page; it is a keyboard stop.
- **Folded:** nothing under Desk on any page. On a Desk session's page the Desk row is highlighted, since that session has no row of its own.
- **N:** Settings › Layout › **Desk sessions in sidebar**, under Sidebar shows Pinned: a pop-up like Terminal font with 3, 4, 5, 8 and All; 5 unless changed. Note: "The ones you used last, under Desk on every page." A hand-edited count is brought into 1–20. The palette finds it (it is in `Row::ALL`).
- **Storage:** app Preferences (`Preferences.desk_rows`), as D787's fast mode; no environment variable. Reverse when the app reads daemon settings at launch: move it into the portable sections of `~/.humata2/config/settings.toml` through SettingsService. Rejected: the app reading a config TOML itself.
- **Goldens:** approved: the trailing ChevronDown on the Desk row in the `desk_side` frames (36–43, 55 and the others drawn by `design/desk.rs`), and 29-settings gaining one row whose pop-up reads "5". Rejected: any diff in frames without Desk sessions, the empty Desk, or the roam channel-desk frames. The folded state and the "K more" row get no design golden.
- **Gives up:** D148's full list on the Desk page; a sidebar that matches the Desk page's sort.
- **Code:** `features/sidebar/src/lib.rs` (`RowKey::Desk`, `RowKey::DeskMore`, `Delegate::OpenDesk`, `SidebarAction::SetDeskLimit`, `desk_more`, `desk_open`, `desk_limit`, `rebuild_desk`, `desk_pick`; `desk_shown` is gone), `features/settings/src/lib.rs` (`DeskRows`, `Row::DeskRowsInSidebar`, `DeskRowsMenu`, `SetDeskRows`), `features/app` (the cap set at launch and whenever Settings changes it; `OpenDesk` switches to the Desk page; `desk_in_sidebar` is gone), `ui/src/sidebar_desk.rs` (Desk row, chevron, "K more"), `ui/src/settings.rs` (the pop-up).
- **Tests:** `features/sidebar:desk_cap_test`, `:desk_test`, `features/settings:desk_rows_test`, `features/app:desk_test`, `desktop:native_test` (`the_sidebar_lists_desk_sessions_on_every_page`, `the_desk_chevron_folds_its_sessions_and_the_row_still_opens_the_desk`, `a_capped_desk_counts_the_rest_and_more_opens_the_desk`), `desktop:design_test`.
- **Reverse:** set the pop-up to All, put the route gate back (`desk_in_sidebar`, `desk_shown`), and drop `Folding.desk`.

## D792: An agent asks the user to import a machine provider through the Inbox; the card shows the whole file and only the user's approval adds it (owner's ask, Fable 5.2's picks, 2026-11-18; amends MP-23; adds an IN-17 row; uses D771, D38.6). Decided (by Fable), built

- **Asked by the owner:** "I also want to be able to have the mcp put smth in my inbox for importing providers because apparently right now it doesnt work."
- **Why it didn't work:** `provider_import` was on the MCP user-only list (HOST_SECRETS), so an agent got `user_only` with advice to use `inbox_ask`, and nothing reached the Inbox. `inbox_ask` only makes a question card, which imports nothing. ProviderService's Preview/Import refuse agents, the Inbox had no preview for an import, and no deferred approve-runs-the-import path existed.
- **MP-23, amended:** only the user adds or changes a provider. Humata never loads a provider from a repo or from any file on its own. The user imports a file in Settings; an agent may only ask, by handing over the file's text, and the ask goes to the Inbox. Either way the import shows the full text, every command each provider runs and the providers it replaces, and it is added only after the user, present, confirms. An agent's ask is never allowed without asking, never Always-allowed, and never part of Approve all.
- **Capability:** `PROVIDER_IMPORT = 49`, "Ask you to import a machine provider". Tier Ask me (no Allow): a rule offers Ask me and Never only, starts at Ask me, no "Always allow …", never low-risk while away, needs the Mac, scope Global. No Desk switch covers it. It is a row in Settings › Agent access like every rule (every new action gets a switch).
- **RPC:** new agent-only `ProviderService.RequestProviderImport {idempotency_key, text, file_name, reason}` → `SetProvidersConfigResponse`. `PreviewProviderImport`/`ImportProvider` are untouched and stay the user's. A user's call to the new RPC is `failed_precondition`, pointing at PreviewProviderImport.
- **Before the Inbox:** text over 64 KiB, a missing reason, or a file with problems (bad TOML, no providers, a provider missing `create`, a merge that doesn't parse) is `invalid_argument` at once, with the problems listed; nothing is asked. A Never rule is `denied` at once.
- **The card:** `RequestPreview.provider_import = 15` (`ProviderImportPreview`: text, file name, providers with labels, every command per provider and action, replaced ids), kind MACHINE, the agent's reason. No plan token is shown. The app shows it as text: adds/replaces lines, the commands with a note that they run on the Mac or on `run_on` when a machine is made, then the file whole. Approve all and Deny all leave it out (`Request::answered_in_bulk`, by its operation id `provider.request_import`).
- **Approve:** the held call writes the merged providers file worked out when the agent asked, only if the file's revision is still the one it was asked against; otherwise the result is `plan_stale` and nothing is written. Asks dedupe by idempotency key (TP-14).
- **MCP:** `provider_import {file_text, file_name?, reason, wait_seconds?}` is a tool now (removed from §7's user-only list); it waits like every asked call and returns `{decision, imported, revision}`.
- **Follow-ups, not built:** the held call isn't kept across a daemon restart (the card is, as for every deferred call); a machine's daemon doesn't forward the ask to the host yet (Fable: forward as a `ForwardedCall`, `host_unreachable` while away, never journal); `humata provider import` from inside a session doesn't ask yet; a Settings import UI.
- **Code:** `daemon/access` (`Capability::ProviderImport`, its policy arm), `daemon/humatad/src/provider_service.rs` (`request_provider_import`, `write_asked`) and `provider_service/ask.rs` (`AskError`, the card), `daemon/humatad/src/mcp.rs`, `daemon/mcp/src/tools.rs`, `daemon/mcp/src/user_only.rs`, `clients/live/src/inbox.rs`, `clients/src/model.rs`, `features/inbox/src/lib.rs`, the three protos.
- **Tests:** `daemon/access:provider_import_test`; `daemon/humatad:provider_import_test` (approve adds it, deny adds nothing, stale after a change, a file with problems asks nothing, Never denies, the user can't use the ask); `daemon/humatad:mcp_test` (`an_agents_provider_import_waits_in_the_inbox_and_approving_adds_the_provider`); `features/inbox:provider_import_test`; `clients/live` unit test for the card; `daemon/mcp:schemas_test` golden.
- **Reverse:** drop the capability, the RPC, the preview and the tool, and put `provider_import` back on the user-only list.

## D795: Codex runs under a `humata codex` wrapper that owns Codex's app-server and links it to the daemon; Codex hooks and their trust step are left for older Codex and user-defined kinds (owner's AGENTS2 direction, Fable 5.2's pick, 2026-11-18; amends D202 and D670; follows D201). Decided (by Fable), not built

- **Asked by the owner:** "I don't like the fact you parse the terminal screen." AGENTS2 reads Codex through its own structured channel.
- **Shape:** the built-in `codex` kind's program is `humata codex`. It starts `codex app-server --listen unix://<session state>/codex.sock` and runs Codex's TUI on it with `--remote`. A second client on that socket, the link, gives the daemon these things:
  - turns: turn/started and turn/completed; an interrupted turn shows as "stopped by you";
  - the thread id, from thread/started, so `/new` is followed;
  - approvals: item/commandExecution|fileChange/requestApproval, as Inbox asks;
  - questions: item/tool/requestUserInput, as Inbox questions with options.
  The first answer wins. Codex's own popup stays as the fallback (verified live on 0.169.0, HOOKS-research.md). Wrapper sessions get no Humata hooks, so they need no trust step.
- **Conditions:**
  - The app-server is the wrapper's child inside the PTY. It lives and dies with the holder, never with the daemon, and the link reconnects after a daemon restart. The state folder is 0700 and the socket 0600.
  - The transport is `tokio-tungstenite` over `UnixStream`, in a single-job crate, added through main.
  - Parsing is tolerant: unknown methods and fields are ignored.
  - If the app-server doesn't come up, or the handshake fails, the wrapper execs plain Codex on the shipped `-c hooks` path with D202's trust row. That path is frozen. Running Codex sessions stay on hooks plus the D670 scan until the owner ends them; none is restarted to migrate. D670 stays for import.
  - The wrapper never changes Codex's approval policy, its sandbox, or the owner's own Codex config or hooks. Codex's exit status passes through, and the TUI is never relaunched on its own.
  - After a move, the wrapper starts a fresh app-server at the new place and resumes the recorded thread.
- **Not chosen:**
  - hooks plus the trust step: the step stays, and the hooks can't answer a question;
  - both channels: the hook surface would be built twice;
  - `--dangerously-bypass-hook-trust`: it also runs a cloned repo's own hooks.
- **Reverse:** point the codex kind back at `codex … -c hooks=` (still built), and delete the wrapper and the link crate.

## D796: Claude Code's own questions (AskUserQuestion) are answered from the Inbox through a PreToolUse hook that returns the answers (AGENTS2, Fable 5.2's pick, 2026-11-18; extends D191's hooks to Claude questions). Decided (by Fable), not built

- **Shape:** a PreToolUse hook matched to `AskUserQuestion` blocks and raises the questions in the Inbox. It returns `permissionDecision: "allow"` with `updatedInput` carrying the questions and `answers`, so the tool returns the person's answer and Claude's dialog never opens. Multi-select and free-text "Other" are both supported.
- **The way out:**
  - The card shows the live terminal and has "Answer in the terminal", which releases the hook with no output so the dialog opens.
  - If the daemon is unreachable, the hook times out, or the card is dismissed, the hook exits 0 with no output and the dialog opens. It never denies on failure.
  - The hook's timeout is long, as for the PermissionRequest hook.
  - The card is withdrawn when the hook process ends, and on Stop, interrupt or SessionEnd.
- **Fallback:** a deny that carries the answer as its reason (Copilot's way), used only if a live check shows that `allow` plus `updatedInput` still opens the dialog.
- **Reverse:** drop the matcher from the Claude hooks file.

## D797: Humata's own sends wait for the agent's structured ready signal instead of reading its screen; after 20 s the Inbox says the agent is waiting in its terminal (AGENTS2, Fable 5.2's ruling, 2026-11-18; amends D762 and D767; uses D781's card form). Decided (by Fable), built (AGENTS2 batch 6)

- **Ready signals:**
  - Claude: a SessionStart hook of any source. It runs only once workspace trust is accepted.
  - Codex: the wrapper's thread/started.
  - Copilot: its sessionStart hook.
  Kinds without a signal keep the screen check. No screen matchers are added.
- **Only Humata's sends wait:** the start message, composer and Inbox sends, and MCP sends. Keys the person types into the terminal always pass straight through. There is no deadline after which Humata sends anyway: a held message stays queued until the signal arrives or the session ends.
- **The card:** 20 s after the holder launches the agent, the session needs you. The Inbox gets "<Agent> is waiting in its terminal", with the live terminal and the held message under it. Its one action, "Send it now", releases the held message by the person's explicit choice; it never answers the agent's screen. The card is withdrawn when the signal arrives, and the queue then flushes in order.
- **Codex:** this rests on a live check that thread/started comes after Codex's trust and first-run screens and before the first prompt. If it doesn't, Codex keeps the screen check for now.
- **Reverse:** make the gate the screen check again for every kind.

## D798: Once an agent's structured channel is heard, the screen fallback stays quiet for its session; each structured ask is withdrawn where it was answered (AGENTS2, Fable 5.2's ruling, 2026-11-18; narrows D786). Decided (by Fable), built (AGENTS2 batch 6)

- **Suppression follows the link, not a held ask:**
  - Once a session's structured channel has been heard, `screen_ask` raises nothing for that session.
  - If the channel drops, the fallback resumes.
  - When the channel returns, the structured card replaces any screen-raised card for the same ask.
- **Observe-only prompts:** a Claude Notification of `permission_prompt` or `elicitation_dialog` with no held ask raises D797's "waiting in its terminal" card, not a screen parse. `idle_prompt` is ignored.
- **When a card goes:**
  - a Codex card on serverRequest/resolved;
  - a Claude card when its hook process ends;
  - every held ask for a session at turn end, interrupt, SessionEnd, Stop or End.
  An Inbox answer that comes after the ask is resolved does nothing, and the card says it was answered in the terminal.
- **Reverse:** let `screen_ask` raise whatever the link holds.

## D799: AGENTS2 follows Claude's conversation on every main-thread SessionStart and Codex's on the latest thread; it starts with three live checks and cuts what the exit doesn't need (AGENTS2, Fable 5.2's ruling, 2026-11-18). Decided (by Fable), not built

- **Live checks before building:**
  1. For Claude, `allow` plus `updatedInput.answers` suppresses the AskUserQuestion dialog.
  2. For Codex, thread/started fires after trust and first run and before the first prompt, and fires again with a new id on `/new`.
  3. For Claude, SessionStart `source=clear` carries the new `session_id` and `transcript_path`.
- **Amended by main, 2026-11-18:** the live checks are owed until the owner can watch, because the approval to copy agent logins ended at 08:20. Each part is built with its fallback wired and tested on recorded fixtures, so a check's result only flips a switch (AGENTS2.md › Lane › Owed: O1 `ClaudeAnswer`, O2 `ReadySignal`, O3 the SessionStart `clear` matcher).
- **Following:**
  - Claude: every SessionStart without `agent_id` sets the session's conversation to its `session_id`, whatever the source. Payloads with `agent_id` never do.
  - Codex: the latest thread/started wins. Its rollout file is found by id under `<home>/.codex/sessions`.
  - Resume always uses the last recorded id.
- **Cut:**
  - new Codex hooks;
  - tracking either agent's subagents;
  - compact handling beyond recording the id;
  - migrating running sessions;
  - a per-session "which channel" UI;
  - new upgrade tests;
  - new screen matchers or recordings for Claude and Codex.
- **Added:**
  - The VERIFY Copilot cases run by agent kind (copilot, claude, codex) rather than as copies.
  - The fast tier uses replayed app-server frames for Codex, and the real hook commands fed recorded payloads for Claude.
  - The live drive runs once, in the slow tier, before release.
- **Order:**
  1. The Codex wrapper and link.
  2. Alongside it, Claude's questions and conversation following.
  3. The ready gate and the doubles rule.
  4. VERIFY by kind, and the live drive.

## D800: A review can be a stack of layers that the owner steps through across repos, only looking, never reviewing (owner, 2026-11-18; picks by Fable 5.2). Decided, planned (STACKVIEW)

- **Asked by the owner:** "If its a stack of changes I want to see it as a stack. I want to see it across the entire stack for all repos involved. … sometimes i just wanna look at the code."
- **Payload:** `review_prepare` takes `stack` or `spec`, never both and never neither. A stack's layers are listed bottom first. A layer has:
  - a trimmed `title`, which is the layer's identity everywhere: unique ignoring case, at most 60 characters;
  - an optional `purpose`: one line, at most 120 characters;
  - one or more repos, each listed once.

  At most 50 layers. Over any limit is refused, stating the limit; nothing is truncated.
- **Range layer:** a two-dot `base..head`, with the revs kept as sent. The commits they resolve to at prepare are stored beside them.
  - Both revs must resolve, and base must be an ancestor of head.
  - In each repo the range layers must chain: each base is the previous range layer's head.
- **Forks are refused.** A forked stack is sent as one review per branch.
- **Worktree layer:** HEAD plus the working-tree files under `include_paths`: tracked changes, and untracked files that gitignore allows. At least one path is required; overlapping paths are allowed.
  - In a repo with range layers, the worktree layers sit above them, and HEAD must be at the top range head.
  - An empty worktree layer is accepted and shows "No changes".
- **Whole stack** is derived by the daemon, never sent. It is the union of the layers: per repo, from the first base to the top head, plus the worktree overlay limited to the union of the worktree layers' paths. To include everything uncommitted, an agent sends `"."`.
- **Whitespace and renames** are set per review and apply to every layer.
- **Drift:** when a kept rev moves and breaks ancestry or the chain, when it is gone (for example after a move), or when HEAD leaves the top head, that layer's repo shows "Moved since it was sent" with one action: ask the agent to send it again. A wrong diff is never shown.
- **Notes:** a note may carry `layer`.
  - A layered note is anchored against its layer's diff. It also shows in Whole stack where its quoted lines are still found.
  - An unlayered note, and a `review_note` sent without `layer`, belong to Whole stack.
- **Order and groups** apply within whichever layer shows; groups are no longer the layer stand-in.
- **View:**
  - The layer list sits at the top of the Diff panel's file list: Whole stack first, then layers 1..N, where 1 is the bottom.
  - Each row shows its pull requests (collapsing to "+N" when they don't fit) and its file count. The purpose shows on the selected row only.
  - At most about 6 rows show; the list scrolls.
  - The base picker is hidden while a stack shows.
  - Repos a layer does not touch are not shown.
  - A stack without notes draws no review chrome.
- **Keys:** `alt-cmd-down` and `alt-cmd-up` step to the next and previous row, with no wrap. The palette gets Next layer, Previous layer and Whole stack.
- **Agent:**
  - `review_focus` takes `layer` and applies at once with Undo (PR-24).
  - `review_context` returns the stack, the layer showing, and per layer the resolved commits and drift.
  - A new `review_prepare` keeps the layer showing if its title still exists, else shows Whole stack.
- **Offers, Undo, Revert and restarts:**
  - Outside a review chat the stack arrives as an offer, "3 layers over 2 repos" (VW-2), which opens at Whole stack.
  - Undo and Revert restore the stack, the layer and the bases as one unit.
  - The stack and the layer survive restarts and moves (PR-26).
- **Not per layer:** Viewed and Mark reviewed stay per file and per repo. Layer diffs are computed only when a layer is picked, and cached by commit.
- **D791's open item:** `review_note` works in any review chat, not only a PR review; a daemon test pins this.
- **Goldens:** new frames 119-diff-stack-whole and 120-diff-stack-layer, using the milestone's three-layer example. Frames 84-86 do not change. Fable approves the pictures once they are rendered.
- **Reverse:**
  - Drop `stack` and `layer` from the tool schema and the proto fields (unused, reserved).
  - Remove `stack.rs` and the layer list.
  - Put the groups stand-in back in the skills.

## D801: Terminals keep running: holders move onto a new build in place, and rename, move, archive, delete and automatic closes keep them running (LIVEHOLDER)

- **Asked:** 2026-11-18, by the LIVEHOLDER manager. Owner rule the same day: "As much as possible I want terminals to keep running."
- **Decided (by Fable 5.2), 2026-11-18.** Amends D373 (shells move too; idle wait only for holders too old to move), D753 (its shell refusal text applies only to pre-LIVEHOLDER shells) and D784 (channel delete and the channel-desk watch).
- **A. Re-exec in place.** Same pid, so the program stays its child (waitpid, exit code, pid file). An `SCM_RIGHTS` handover to a new process is rejected: the program would be reparented. Amendments:
  - Negotiate `--handoff-versions` before quiescing.
  - Bound the check at about 2 s, and kill and reap the check child on timeout.
  - Reap every child by its pid, never `waitpid(-1)`.
  - In the new image: reset the signal mask and dispositions, then `waitpid(child, WNOHANG)` once.
  - Clear CLOEXEC on the listener and master only after the check child has exited, and set it again if the exec fails.
  - Refuse `MoveTo` while a keyed input or `AwaitKeys` is pending; the daemon retries.
  - Send each client `Moving` before the exec, so it reconnects at once and resends un-acked keyed input under the same keys.
  - Carry everything Welcome serves: terminal modes, any partial escape or UTF-8 tail, and `holder_generation`.
  - Welcome reports the build. The daemon rewrites `holder_builds` only after it reconnects and sees the new build.
  - fsync `.handoff` before the check. The old image deletes it on any refusal; the new one deletes it once serving. `.handoff` with a dead pid means lost in the move; with a live pid it means in progress, and revive leaves it alone.
- **B. Crash safety: a precheck, the marker and a report. No keeper process.** A keeper can neither serve nor report the exit code.
  - `--check-take-over` runs the real take-over code minus adopting the fds.
  - Migrate moves one holder first as a canary (an idle one if any), confirms its reconnect and Welcome, then moves the rest. It stops at the first failure and marks the item Failed.
  - A loss reads "lost while moving to the new holder build": scrollback kept, agents offer Resume, shells read ended.
- **C. Migrate moves agents and shells with no idle wait** (subject to A's busy retry).
  - The item's body says it moves N terminals without restarting them.
  - It lists separately only holders too old to move. Their agents restart once at an idle moment (D373). Their shells are never restarted or ended and never hold the item open.
- **D. Channel rename keeps agents running.**
  - Rename the folder and rewrite the records at once.
  - Each running agent keeps its started-under path and conversation key with a pending re-key flag. Every stop/resume path re-keys from the original key to the current path before resuming. Agents not keyed by cwd skip it.
  - An old→new alias symlink lives while any holder started under the old path runs:
    - it is recorded with the channel and re-pointed on a later rename;
    - it is removed when the last such holder ends, and at reconcile;
    - placement treats its name as taken;
    - Bin, copy and delete walks never follow it.
  - Swap atomically where possible (`renamex_np(RENAME_SWAP)` / `RENAME_EXCHANGE`); otherwise rename, then symlink.
  - A mid-turn agent needs no confirm.
  - Cross-filesystem renames keep today's stop/resume, and the confirm says why.
  - No "pending re-key" status is shown.
- **E. Desk move to a channel on the same filesystem** is a rename with D's handling. Terminals keep running; undo is the rename back. Across filesystems today's copy stays, and its confirm says they restart. Same-machine MoveSession stays refused.
- **F. LIVEHOLDER builds D784's Archive ask.**
  - `ArchiveSession{end}`: unset keeps it running. An agent caller needs the destroy switch to end.
  - The archived session leaves the sidebar and stays on the machine page as "archived". Open attaches it without restoring it.
  - Confirm: Cancel, red-text "End and archive", blue "Archive" (Return). End is never focused.
- **G. Desk close.** Hide is a view action and never calls `desk_service/close.rs`. Only End or End for good does, and an agent caller needs the destroy switch.
- **H. Channel delete (amends D784).** The confirm's default is to move the live sessions to the Desk and keep them running.
  - Confirm: Cancel, red-text "End them and delete", blue "Move N to the Desk and delete" (Return).
  - The move is E in reverse. Stopped and exited sessions go to the Bin.
  - A terminal that can't move (shared channel folder, cross-filesystem) is named with the reason.
  - An agent caller's delete moves them; End needs the destroy switch.
  - For an offline machine, the choice is carried to the next link.
- **I. Automatic ends.**
  - The channel-desk watch never ends a live holder: it detaches the session to "Not in a channel".
  - `carry_service` release leaves the old machine's shells running under "Not in a channel"; the move confirm names them.
  - These stay, each with its reason:
    - `terminal.rs:1024` rolls back Humata's own failed create; nothing ran.
    - Revive's stale Launching/Resuming undoes a start Humata never finished. It matches on record state only and never on a live `.handoff`.
    - The stopped-boot sweep finishes the owner's Stop.
    - Remove machine is explicit and named.
- **J. Cross-machine move: freeze, don't overlap.**
  - Order: freeze the old agent (new holder `Freeze`/`Thaw` frames: SIGSTOP/SIGCONT to the groups `end_by_pid` targets; the holder serves the frozen screen; the running list reads "Paused for move") → carry → resume on the new machine → confirm → end the old one. On any failure, thaw it.
  - The move note records the freeze, so revive thaws or completes after a crash.
  - Shells are never frozen.
  - A holder without `Freeze` is refused as Outdated, naming Migrate.
- **K. PER-6 now reads:** "A running terminal's program is never restarted to change its holder's build. A new holder build takes each running holder over in place: same process, same program, same screen and scrollback, and viewers reconnect by themselves. A take-over that can't be proven safe is refused before it starts and the terminal carries on as it was. A holder that can't hand itself over keeps running on its own build; its agent restarts once, at an idle moment, only after the owner clicks Migrate, and its shell is never restarted."
  - New sibling rule PER-32: "Humata ends a running terminal only when the owner ends it (End, End for good, Stop, the End choice in a confirm, removing its machine) or to undo a start Humata itself never finished. Rename, move, archive, close and upgrade keep it running, or say why they can't before acting."
- **Reverse:**
  - Holders: drop `MoveTo`, which returns Migrate to D373's restart.
  - Each audited path: put back its stop/resume.

## D803: A stack review can be read from the repo's own stack metadata, so the agent no longer spells out the layers (owner, 2026-11-18, to spend fewer tokens; picks by Fable 5.2; amends D800). Decided, planned (STACKVIEW T3b)
- **Owner, 11:35:** "Are we tastefully basing our diffs off of an internal data metadata and then translating this to GitHub stacked PRs and stuff? This way we could possibly spend less tokens on it."
- **This reverses two lines of STACKVIEW's first plan:** "Humata does not run `stack` itself" and "changes to `tools/stack`" being out of scope.
- **What the agent sends:** `review_prepare` takes `from_stack: {repo, tip?, topics?: {<topic>: {title?, purpose?}}}`.
  - The repo alone is enough.
  - The agent names the repo, never a path. The daemon reads the stack in the calling session's checkout of that repo.
  - An unknown topic key is refused, and the error lists the real topic names.
- **Mixing with explicit layers:**
  - Explicit `layers` may follow the stack's layers: other repos, or a not-yet-committed worktree layer on top. D800's checks run over the whole list.
  - An explicit range layer in the from_stack repo is refused: "make it a topic, or send that repo as explicit layers".
- **How the daemon reads it:**
  - It runs `stack status --json --offline` and turns each topic into a layer, in chain order, with the range `base_commit..head` that the tool reports.
  - It keeps each layer's head alive under a ref of its own, because a rebuilt topic's head may have no other ref.
  - One repo per call. Topics are never joined by name across repos.
- **Titles:**
  - A layer's title is the subject of the topic's first commit.
  - The topic name shows beside it as written, never humanized.
  - The purpose is empty unless the agent passes one.
  - A layer's identity is its topic name.
- **Forks:** a stack that forks is refused, naming its leaf topics, unless the agent passes `tip`. The layers are then the chain from the root to that tip.
- **Merged topics:** no special handling. A topic whose commits are still local is a layer.
- **Network and GitHub:**
  - Reading the stack needs no network and no GitHub login.
  - The PR number, link and Merged chip come afterwards from a best-effort online `stack status --json`. If that fails they are simply absent, and nothing is said.
  - Opening and updating PRs stays the stack tool's job, from the same metadata.
- **Refusals, each saying what to do:**
  - A restack is in progress: "finish or abort it, then send again".
  - No commit between trunk and HEAD carries a Topic trailer: "add trailers, or send explicit layers".
  - More than 50 topics: D800's limit.
  - The stack tool is missing or too old: also raises the VIP Inbox "Migrate stack CLI on <machine>", per the force-upgrade rule. Nothing is guessed from git in its place.
- **Drift:**
  - It re-reads the stack the same way and matches layers by topic name.
  - A topic whose head or base_commit changed is Moved.
  - A topic that is gone, or no longer on the tip's chain, is Missing.
  - New topics above the sent tip are HeadMoved.
  - A re-read that hits a restack keeps the last reading and shows nothing.
- **Refresh:** refreshing a moved from_stack review is done by the daemon, without the agent. It adopts new ranges, appends new topics and drops missing ones. It keeps the state of unchanged layers, and the agent's titles and purposes by topic name.
- **The tools/stack change, additive only:**
  - `head` and `base_commit` per topic in `--json`, from `local::rebuild`'s `Tip`.
  - An `--offline` flag: no fetch and no gh. Trunk and remote come from config and the remote-tracking ref, the merged set is empty, and the GitHub fields are null.
  - `copy_onto` keeps the source commits' author and committer identity and dates, so a rebuilt topic gets the same commit ids on every run. A test checks that two runs of status give the same heads.
- **Reverse:** drop `from_stack`. Explicit layers are unchanged. The tools/stack additions can stay.

## D802: An agent can say which files matter and order each layer's files (owner, 2026-11-18; picks by Fable 5.2). Decided, planned (STACKVIEW T3c)
- **Owner, 11:40:** "Also we want AI to be able to order diffs and files based on importance."
- **What's new:**
  - A review plan, and each layer of a stack, can carry an importance list: files marked key or skim, everything else normal. This is cheap for the agent: paths and a tier, no prose.
  - Each layer can also carry its own order and groups, as the plan already does.
- **Numbering:** this entry follows D803 in the file because D803 was written first, while these rulings were pending.
- **Wire:**
  - A new message, `ReviewImportance {RepoPath file; Tier tier}`, with tiers key, skim and normal. `RepoPath` is unchanged.
  - Fields:
    - `ReviewPlan.importance = 7`
    - `ReviewLayer.order = 5`, `.groups = 6` and `.importance = 7`
    - `FileSummary.tier = 10`
  - These are Fable's shapes, renumbered around D803's `ReviewPlan.from_stack = 6` and `ReviewLayer.topic = 4`.
  - The same file twice in one list is refused. A path not in the diff is treated the way `order` treats one.
  - The plan stores only what the agent sent. The daemon works out the final tier on each `FileSummary` it serves.
- **Defaults:**
  - An unlisted file is normal.
  - The daemon marks some files skim by itself:
    - lockfiles by name (Cargo.lock, cargo-bazel-lock.json, MODULE.bazel.lock, package-lock.json, yarn.lock, pnpm-lock.yaml, go.sum, Podfile.lock);
    - paths that .gitattributes marks `linguist-generated`;
    - renames with no content change.
  - It never reads file content to decide.
  - These marks are worked out when the files are served and are never stored.
  - The agent's tier wins either way. TIER_NORMAL un-skims a file.
- **Key files:**
  - They carry a small filled accent dot before the name, in the file list and the diff header, with a "Key file" tooltip. There is no label or badge.
  - They are not reordered when the agent sent an order.
  - With no order sent, today's order stays, except that key files come first within each repo, keeping their relative order.
- **Skim files:**
  - They draw last in their repo as one folded row, "12 files to skim". Click, Return or right-arrow opens it; left-arrow folds it.
  - Opened, they show dimmed, with their diffs collapsed to the header until clicked.
  - They leave their groups, and a group left empty draws no header.
  - The files total still counts them.
- **Keys:** while the skim row is folded, next and previous file (alt-cmd-down and up) treat it as one stop and skip its files. Once it is opened, they step through them. There is no new shortcut.
- **A picked layer** uses its own order, groups and importance, and nothing from the plan.
- **Whole stack** uses the plan's when they were sent. When they weren't:
  - Order: the layers' orders joined, bottom layer first, each file at its first appearance.
  - Groups: none.
  - Importance: a file is key if any layer says so, and skim only if every layer touching it says so.
- **Repos** are ordered by where each first appears in the order. There is no separate field.
- **Look-only stacks** show all of this, because it helps reading and is not review chrome. Plain reviews, without a stack, behave the same.
- **Not now:** ordering hunks inside a file is a follow-up. Hunks stay in file order, and the agent points at one with a note.
- **Frames:**
  - 119 (Whole stack) gains the folded row, and 120 (a layer picked) gains a key dot and that layer's own order.
  - A new frame, 121-diff-stack-skim-open, shows the fold opened.
  - Fable approves all three once they are rendered.
- **Reverse:** drop the importance lists, `FileSummary.tier`, and the layers' order and groups. The plan's order and groups are unchanged.

## D804: Stack layer keys and the wire shapes for D800, D802 and D803 (STACKVIEW manager, 2026-11-18; keys picked by Fable 5.2). Decided
- **Keys (Fable 5.2):** alt-cmd-] is Next layer and alt-cmd-[ is Previous layer. Next goes Whole stack (row 0), then layer 1 (bottom) up to N, with no wrap; Previous stops at Whole stack. Both work only in the Diff panel while a stack shows. "Whole stack" is a palette entry only; cmd-] stays unbound. Both chords are on the shortcuts sheet.
- **No `DiffBase::Layer` in the app.** A layer or Whole stack is a `DiffBase::Spec` whose specs use the new `RepoBase::Range{base, head}` or `RepoBase::Overlay{base, paths}`. Such a spec shows exactly its repos, so the live client narrows `GetDiffRequest.repos` to them. This keeps the Diff panel's existing base matches unchanged.
- **Field numbers:** `FileSummary.tier` is 12, because 10 and 11 hold the file modes. `ReviewChange.layer` is 12 (empty means Whole stack).
- **MCP `from_stack.topics`** is a list of `{topic, title?, purpose?}`, because tool schemas have no map type. humatad turns it into the proto map; a topic listed twice is refused.
- **Titles read from a stack** are the first commit subject cut to 60 characters. A subject that repeats an earlier layer's title falls back to the topic name.
- **Reverse:** drop the keys, the two RepoBase variants and the tier and layer fields.

## D805: Where a file's tier comes from in the app, and how the daemon keeps it (STACKVIEW manager, 2026-11-18; amends D802). Decided
- **Daemon:** GetDiff serves each file's final tier in `FileSummary.tier`: the agent's tier from the review chat's current view, else its own (lockfiles and generated files skim). The view's tiers come from the picked layer's list, else from the plan's. For a stack's Whole view with no order, groups or importance sent, humatad fills the plan's order and importance from the layers joined (D802 rules), filtered to the files the diff has.
- **App:** a plan can arrive (as a ReviewChange or a draft view) before the diffs are read again, so `ReviewState::tier_of` reads the agent's tier from `PlanView.importance` or the picked `LayerView.importance` first, then `FileDiff.tier`.
- **Kept:** `ReviewViewState.importance = 9` stores the plan's tiers with the review, so they survive a restart and Undo. The app reports it empty and the daemon keeps its own, as the app never changes tiers.
- **Refused:** a plan or layer listing a file twice in `importance` is refused with the field named.
- **Reverse:** drop field 9 and the two `importance` lists in the app; tiers then come from the diff only and lag a plan until the next read.

### D-REVIEWUI-1: reopening a review reads its diff again after 60 s (provisional, 2026-11-18)

- **Rule:** reopening the same session's review reuses the held diff if it was read less than 60 s ago, and reads it again otherwise, so terminal edits made without a turn ending still show.
- **Reverse:** change the window in `features/review/src/reopen.rs`, or always read again.

## D806: How `humata codex` wraps Codex (AGENTS2 manager, 2026-11-18). Provisional

- **Flags:** the app-server gets only the `-c` config pairs; `--model` and every other flag go only to the TUI.
- **Socket:** a fresh 0700 folder `h2cx-<16hex>` under the temp dir (or /tmp if the socket path would pass 100 bytes), removed when the session exits.
- **Hooks:** in remote mode no hooks are passed, so asks never arrive twice, and the wrapped codex shows no D202 trust row. In the plain-codex fallback, Codex itself asks once in /hooks to trust them.
- **Finding codex:** only the env-wrapped built-in kind is looked up by name on PATH; a custom codex kind is found by its own program path.
- **Owed live check (O4):** `codex app-server` accepts the `-c` passthrough and `codex --remote unix://S resume <id>` works. If not, drop the config pairs or fall back to plain resume.
- **Reverse:** point the codex kind back at plain `codex` in `daemon/sessions/src/agents.rs`.

## D807: Progress indicators: one thin bar, where the action was started (PROGRESS manager, 2026-11-18; picks by Fable 5.2). Provisional

- **P1 One thin bar, no spinners.** A 2 px line with three forms (measured, unmeasured sweep, elapsed text). Reason: quiet chrome; one part reused everywhere. Reverse: swap the widget's drawing.
- **P2 Progress sits where the action was started or on its subject**, never in a global tray. Reason: actionable UIs only; a list of running things the person can't act on is status for its own sake.
- **P3 300 ms before anything draws; elapsed text after 10 s.** Reason: no flicker on fast operations, no ticking clock on short ones.
- **P4 The running step's title, not "step 2 of 5".** Reason: step counts change with skipped and undo steps and say less than the title.
- **P5 Clone units are 0–1000 across git's phases (receive 0–900, deltas 900–1000; local clones use Updating files).** Reason: one rising number per step fits `set_units`' only-rises rule; object counts and file counts can't share a total. If git's output can't be parsed, the step is unmeasured, never an error.
- **P6 Unit updates are coalesced to 4 per second per step in the daemon.** Reason: performance; a clone prints hundreds of lines.
- **P7 Archive push, restore download and trash stay unmeasured in this milestone.** Reason: each needs its own honest total; add when someone finds them slow.
- **P8 A turn's elapsed time comes from a new `Session.status_since`, shown only in the session tab header and only while Working.** Reason: `last_output_at` is not a turn start; the sidebar stays still (no per-second redraws across the list).
- **P9 A stalled operation shows nothing new.** Elapsed keeps counting and the existing Cancel stays; no "seems stuck" label. Reason: no unfixable detection.
- **P10 The diff indicator waits for DIFFVIEW; the holder count waits for LIVEHOLDER T6.** PROGRESS can close and release without T6/T7 if either owner is still open, carrying them as follow-ups in those milestones.

## D808: An agent closes a Desk session or deletes a channel only with the destroy switch on (LIVEHOLDER manager, 2026-11-18; follows D801 G and H, PER-32). Provisional

- **Gate:** for an agent, CloseDeskSession, PlanDeleteChannel and DeleteChannel need MachineDestroy on this machine, which is Never by default. The user's own rules (the ChannelDelete tier, present user) stay unchanged, and a peer is still refused.
- **Why this switch:** closing and deleting with `end_live` end running terminals. Reusing the one destroy switch means the Agent access settings and goldens don't change.
- **Nothing asks the Inbox:** with the switch off the call is refused (CH-25 still holds), not turned into an approval request.
- **Reverse:** put `user_only` back in `desk_service.rs` `close_desk_session` and drop the agent branch in `channel_service/delete.rs` `user_deletes`.

## D809: AGENTS2 batch 6 picks for D797/D798 (provisional, 2026-11-18)

- **Send wait:** a Humata send (composer, Inbox reply, MCP `session_type`) waits at most 30 s behind a held start message. After that it is refused with FailedPrecondition and nothing is typed. The person's own keys are never held.
- **Gap card:** after a Codex link drops, a screen card raised during the gap stays until its screen stops asking. Nobody has checked live whether Codex sends a pending ask again on resume (O4).
- **Notice card:** "<Agent> is waiting in its terminal" is a plain card with key `notice-in-terminal`, not the D781 form, because hooks.rs has no tab index. The UI should offer only Dismiss for that key.
- **No notice** when the person keeps that agent's asks in its terminal.
- **Idle sweep:** SessionStart's idle report also withdraws held asks.
- **Reverse:** drop `behind_start` in `daemon/humatad/src/start_ready.rs` and the gate in `terminal.rs` `type_in`; drop `raise_notice` and its callers in `session_service/hooks.rs`.

## D830: How the To-do list ranks (BOARDSIMPLE manager, 2026-11-19; follows the signed-off frames and the xhigh review). Provisional

- **Order:** pinned items first, in pin order (dragging among pinned reorders them); then score = due urgency + value per effort (impact ÷ effort) + waiting time for review requests; ties by the migrated starting order (In progress, In review, To do, Backlog; then old position; global board before channel boards), then older first, then key.
- **Due urgency:** overdue 100; today 90; 1–7 days 80 − 5/day; 8–30 days 30 − days; later or none 0. **Value:** 10 × impact (High 3, Medium 2, Low 1) ÷ effort (S 1, M 2, L 3; unset counts as M); no impact, no value. **Review request:** 60 + 10 per day waiting, at most 100.
- **Age never adds score** (xhigh review: the list must not change each morning for no reason a person can name); it only breaks ties, so a bare list reads oldest first.
- **Reasons** say only what the row doesn't: Pinned (· was VIP), High impact (for a small job), "<item> waits on it", "Waiting on you for N days", and for items with nothing set "Was 1st in To do" or "Added N days ago". Due is a chip, never a reason.
- **Do next** is the first row of Next among the rows shown, whatever the filter.
- **Rules live in one crate**, `apps/humata2/todo`, used by the app and the daemon.
- **Reverse:** change the weights in `todo/src/rank.rs`.

## D831: Groups, the overdue rule and the open count (BOARDSIMPLE manager, 2026-11-19). Provisional

- Three states: Open, In progress, Closed. Waiting on another item and Hidden until a date are derived folds, not states.
- Group precedence: Needs attention (an agent's ask, or a suggested item) → Closed (shown for 7 days with Reopen; older ones only in History) → In progress → Next. **An overdue open item is always in Next**, never folded into Waiting or Hidden until (owner rule).
- A suggested item has no rank until the user says Yes.
- The header's "N open across M channels" counts items not closed, not suggested and not hidden; channels are the distinct channels among them ("No channel" doesn't count).
- **Reverse:** `todo/src/rank.rs` grouping.

## D832: The To-do API is a new TodoService beside BoardService (BOARDSIMPLE manager, 2026-11-19). Provisional

- New messages and a new `TodoService` in `boards.proto` (TodoItem = BoardItem + TodoFields; ListTodo, CreateTodo, UpdateTodo, NextTodo, Get/DismissTodoMigration). `common.proto` and `BoardColumn` are untouched; migration reads `BoardColumn` into `TodoStart`.
- **Why:** adding fields to `BoardItem` or the board requests breaks every struct literal of them (about 60, across other lanes' tests and the daemon). A new service is additive: nothing builds differently until the daemon serves it.
- BoardService is retired once the app and MCP move to TodoService (UPGRADE: one Migrate message, no mixed-version fallback).
- **Reverse:** drop the TodoService block at the end of `boards.proto`.

## D833: Moving the boards into the list (BOARDSIMPLE manager, 2026-11-19). Provisional

- Backlog and To do → Open; In progress and In review → In progress; Done → Closed (closed at its done time). Items of archived sessions' boards migrate Closed, so D216's hiding holds.
- Each item keeps its key, history, sessions and dependencies, and records where it stood (`TodoStart`: column, 1-based position, board, VIP). That order is the starting rank; nothing else is invented: no dates, effort or impact.
- VIP becomes Pin, VIP items first; the row says "Pinned · was VIP".
- One Migrate card (frame b6-3) with counts and how to restore; the day after, one banner ("Your boards became this list yesterday…", frame b9-1) until Got it.
- **Reverse:** the boards stay readable through BoardService until it is retired.

## D834: The sidebar rename lands in one re-record with lane INBOX (BOARDSIMPLE manager, 2026-11-19). Provisional

- "Board" → "To-do" and "Inbox" → "Agent inbox" change the sidebar of nearly every design golden. The To-do frames show the new names through an opt-in sidebar flag; the global rename is one batched re-record with INBOX's frames, judged once by Fable.
- **Reverse:** drop the flag.

## D835: How the To-do list keeps what the board store cannot (BOARDSIMPLE manager, 2026-11-19). Provisional

- Due date, effort, impact, tags, steps, Hide until, pin order, list state and channel moves live in a journaled sidecar (store `settings`, key `todo.fields`) next to the board items, so the store schema is unchanged.
- An item's revision is the store revision plus its sidecar edits, so two clients editing a due date conflict like any other edit. Sidecar-only edits bump the revision but are not in the item's History yet.
- A list state holds while the item stays in the column it was set in; Close writes Done and Reopen moves it back, so the old boards agree until they are retired.
- `pin_order` is 1-based on the wire; 0 means unpinned.
- **Reverse:** move the fields into store columns in a schema milestone and drop the sidecar.

## D836: Days, closed items and suggestions (BOARDSIMPLE manager, 2026-11-19). Provisional

- "Today" is the UTC day on the daemon and the local day in the app; the daemon has no time zone for the owner yet.
- Show closed lists items closed in the last 7 days.
- An item an agent adds is always Suggested and never in Do next until the owner says Yes; Dismiss closes it. Agents cannot accept suggestions.
- An agent's close is an ask governed by its Edit rule; when the rule asks, the item shows Needs attention.
- **Reverse:** change the window or the agent rule in svc_board.

## D837: Migration runs once, VIP becomes pins (BOARDSIMPLE manager, 2026-11-19). Provisional

- `migrate_boards(vip, now)` runs at daemon start-up and lazily on first To-do use; after the first run it is a no-op. The daemon does not store VIP marks, so a start-up call without marks migrates with no pins, keeping the owner's board order as the starting rank (D833).
- Session-board items migrate by view: their channel and session are derived when listing, and nothing is rewritten.
- **Reverse:** clear `todo.migration` to run it again.

## D838: TodoService is host-only for now (BOARDSIMPLE manager, 2026-11-19). Provisional

- An agent on an away machine gets `not_built` for TodoService calls; forwarding needs new `Method` rows in svc_peer's ports and lands later.
- **Reverse:** add the Method rows and an away-board test.

## D839: Agents reach the To-do list through three MCP tools (BOARDSIMPLE manager, 2026-11-19). Provisional

- `todo_next` (read), `todo_add` (always lands Suggested; the owner says Yes) and `todo_close` (an ask under the agent's Edit rule). Dates are strict YYYY-MM-DD; effort s|m|l; impact low|medium|high.
- Until main adds their own Capability values (seam: `TODO_SUGGEST`, Default Allow; `TODO_CLOSE`, Ask me; appended in capabilities.proto and access/capability.rs, named in mcp tools.rs, shown as Settings agent-access switches), `todo_add` rides `ChannelBoardAdd`/`OwnBoardWrite` and `todo_close` the board Edit rule, like `board_item_create`/`board_item_update`.
- **Reverse:** remove the three tool rows from tools.rs and svc_mcp/src/mcp/todo.rs.

## D840: The To-do access switches are their own capabilities (BOARDSIMPLE manager, 2026-11-19). Decided

- `CAPABILITY_TODO_SUGGEST = 72` (Default Allow) and `CAPABILITY_TODO_CLOSE = 73` (Ask me), appended in capabilities.proto and daemon/access; supersedes D839's interim board rules. svc_board checks them for an agent's adds and closes on top of the board rules; the owner's own close is never gated. The fake rules fixture lists both rows.
- **Reverse:** drop the two values and their arms; agents fall back to the board rules.

## D841: To-do list view details (BOARDSIMPLE manager, picks by Fable 5.2, 2026-11-19). Decided

- What changed shows inline under the row, not in a popover.
- A group shows 10 rows, then "Show all".
- The check circle closes an open item in one tap (the owner's own close; it is undoable from Show closed).
- Pin maps to the 1-based `pin_order` of D835; pinned items rank above computed ones, in pin order.
- An item's ask maps to `AskDetail` (evidence text and link, affects, change, asked minutes ago, session ended); Ask why opens the Inbox request, Open session switches to it, History opens the item's history. The app handles these three actions, not the feature.
- **Reverse:** view-only; change the To-do UI and its goldens.

## D845: Screen dialogs never reach the Agent inbox; the session shows "needs you" instead (INBOX manager, picks by Fable 5.2, 2026-11-19; INBOXHOOKS). Decided
- **Cut (superseded by INBOXHOOKS):**
  - D767: a screen question raises an Inbox card;
  - D781: the live terminal under the card;
  - D785: cut-off choices;
  - D786 Pick 2: the one-time grow to 50 rows;
  - the screen-read "waiting" cards that offer only Dismiss.
- **What remains of these features:**
  - A dialog read off an agent's screen holds the session at NEEDS_INPUT (sidebar row and tab) until a look sees it gone, with no Inbox item. This is the path every agent took with D782's switch off, and it is now the only path.
  - No bell mark: D424 means the program rang BEL, which these dialogs don't.
- **Amended:**
  - D782: the per-agent "Ask in the Inbox" switch stays for hook-raised permission asks and ask-user forms. Screen dialogs are always terminal-only.
  - D786: holders keep starting at 80x50 (Pick 1). Resizes come only from attaches that can type (D14 as before D786).
- **D762 stands, frozen.** The input gate keeps refusing typed text while a dialog shows or the screen can't be read. It gets no new patterns, so D762's Codex and Claude Code first-run follow-up is dropped.

## D846: A to-do ask is not an Agent-inbox card (INBOX manager, 2026-11-19; INBOXHOOKS, BOARDSIMPLE). Decided
- **What counts as a to-do ask:** an agent's board/to-do write that a rule sends to Ask me, i.e. a card whose preview is `BoardChange` or `NewBoardItem`. It answers the agent `approval_pending` at once, so the agent carries on.
- **Where it shows:** `ListRequestsRequest.agent_inbox = true` leaves to-do asks out of `requests` and `pending_count`. The Agent inbox, its badge and its notifications use that list. To-do shows the asks on their item (`BoardItem.pending`, Needs attention) and answers them through `InboxService.AnswerRequest`.
- **Reverse:** stop setting `agent_inbox`.

## D847: Card wording and undo windows as drawn (INBOX manager, 2026-11-19; INBOXHOOKS mockups b2–b3). Provisional
- "Always allow for this session" (moves) and "Always allow between these two sessions" (copies) keep IN-14's grant: tied to the asking session and the exact target, and ended with the session (IN-22).
- Undo windows are 7 days for file moves and copies and 30 days for trash, as drawn. They stay provisional until the owner sees them in a release.

## D848: Every Diff repo row says "against X" in words; full screen says Exit full screen and offers Close; stack rows name their repos; Back is a ghost everywhere (DIFFUI manager, 2026-11-19; owner's UI marks, UXSIMPLE 1-2 and 2-2, DS-58; reverses D739's chip pick A). Provisional
- **Repo base:** the chevron-only chip (D739 pick A, D740) is gone. Each repo row reads "against main" or "against release/2.4" in secondary text with no chevron, as UXSIMPLE 1-2 draws it, and a click still opens the per-repo menu. The chip gives way before the repo name but keeps at least 52 pt, so "against" stays in view.
- **Full screen:** "Show the terminal" now reads "Exit full screen" (UXSIMPLE 2-2). The header also keeps the X Close, as the owner's mark asks, although frame 2-2 draws no Close. Close leaves full screen and closes the panel.
- **Stack layer rows:** a layer row names the repositories it touches ("ledger · infra"), and Whole stack reads "5 repos · 23 files", as UXSIMPLE 2-2, 3-1 and 3-4 draw them. The pull-request and file counts on layer rows are gone; the frames draw neither. `StackRow` gains `repos`.
- **Back:** both the session header's and the top bar's Back are the 24 pt ghost button (DS-58). The top bar's was the filled `ghost_in`.
- **Re-recorded:** the native frames that draw a session header, the Diff panel or full screen. Each has Fable's verdict in the DIFFUI log (NATIVEFIX.md).
- **Reverse:** put back the chevron-only chip in `ui/changes/src/panel_changes/review_header.rs`, "Show the terminal" in `ui/core/src/chrome.rs`, and the pulls/files labels in `ui/changes/src/panel_stack.rs`.

## D849: REVIEWER ships as "New agent tab ⌄" in the full-screen Diff header (DIFFUI manager, 2026-11-19; UXSIMPLE_REVIEW §3, frames 3-3 and 3-4). Provisional
- **What it is:** the full-screen Diff header gains New agent tab ⌄ before Exit full screen. Beside the terminal the strip's `+` already does this, and 3-4 draws no such button there. Its menu reads:
  - the frame's one-line explanation;
  - Terminal;
  - a rule;
  - the agents Settings offers (AGENTSET, D725), each with a green dot and named by its command.
- **A pick:** closes the menu, leaves full screen and opens a fresh tab in the session's focused tab row (`OpenTerminal`), so the Diff sits beside it as 3-4 draws. The pick is marked "last used" until the app restarts (not saved). Escape closes the menu before it leaves full screen. There is no dock, no second tab strip, no new session kind and no "Send as mine" draft.
- **Not yet as drawn:**
  - **"codex · not installed":** the daemon does not report whether an agent's CLI is installed. Until it does, the menu lists only the agents Settings offers. Follow-up for main (daemon: report CLI presence per agent).
  - **⌘T:** built by D857.
  - **Opening prompt** and **"not installed":** ruled by D857, not yet built.
- **Reverse:** drop `ui/changes/src/panel_changes/agent_tab.rs`, `diff_header`'s actions, `panel_header_actions`, and `ReviewState::{agent_tab_menu, last_agent_tab}`.

## D850
Fable's step 8 order, and the footer reads "Send to claude" (DIFFUI2 manager, 2026-11-19; UXSIMPLE step 8 (a), frames 3-1 and 3-2). Provisional
- **Order (Fable 5.2):** (a) footer, (c) Looks good and "Since I last looked", (b) three comment states, (d) one agent note card, (f) fold order-humata-review into "show with notes". (e) the Fork dialog is deferred until a design frame for it is drawn and signed off.
- **What it is:** the Diff footer's send button is "Send to <agent>", named by the session agent's command, with no count. It always shows for a session and is disabled when there's nothing to send. A secondary "N drafts" sits to its left. A pull request review keeps "Submit review…" as its one filled primary. Fable ruled that a PR review shows "Send to <agent>" only when something is sendable, as a neutral button to the left of Submit. When nothing is sendable, the disabled button is grey and non-primary, at the same size.
- **Destination:** the session's own agent. The comments already go there; there is no separate "last-focused agent tab" target.
- **Reverse:** restore the hidden-when-empty "Send N comments" in `panel_changes/mod.rs`'s footer.

## D851
Comments show three states, and an addressed comment can be reopened (DIFFUI2 manager, 2026-11-19; UXSIMPLE step 8 (b), frames 3-1, 4-1, 4-2, 4-3). Provisional
- **What it is:** the comment status maps onto three looks, without changing the stored status:
  - Draft and Waiting read "Draft".
  - Sent reads "Sent".
  - Addressed reads "Done by <agent>", with the agent's last reply and Reopen.
  - Resolved reads "Done by you" (or by the agent that resolved it), with Reopen.
  - A file whose shown comments are all done folds them into "✓ N comments done on this file" with Show. The fold is view-local, not saved.
  - The file row reads "✓ N done".
- **Reopen:** the review ledger (`review_diff`) and the fake now reopen an Addressed comment as well as a Resolved one, because 4-2 offers Reopen on "Done by claude". The Addressed card's Resolve button goes; the comment is already shown as done.
- **Not yet as drawn:** settled by D855: "Sent · the agent changed these lines" and Archive → Hide are dropped, and there is no "Show done" control; resolved comments always load and fold as done.
- **Reverse:** restore the old status chips in `ui/changes/src/comment_cards.rs` and drop `Addressed` from `reopen`'s accepted states in `daemon/review_diff/src/comments.rs` and `clients/src/comments/fake.rs`.

## D852
Looks good replaces Mark reviewed, per file and per layer (DIFFUI2 manager, 2026-11-19; UXSIMPLE step 8 (c), frames 2-2, 3-1, 3-2, 4-1, 4-2, 4-3). Provisional
- **A look:** the review state keeps, per session, which files the person looked at, each with a fingerprint of the diff it showed (a hash of the file's diff lines). A file stays ticked only while its fingerprint matches, so ticks clear when the content changes. The host keeps them like the D183 bookmark:
- **RPCs:** `DiffService.GetLooks` and `SaveLooks` (user-only, `UserReviewState`). A save replaces the session's whole set.
- **Storage:** each repo of the session keeps one JSON blob at `refs/humata/looks/<session>`, written by `hash-object` plus `update-ref`. An empty save deletes the ref.
- **Reducer:** the reducer reads looks once when it opens a session, and saves after Looks good, a tick or a clear. A failed save keeps the tick and shows the review error.
- **Fingerprint:** FNV-1a, so it is stable across processes.
- **Layer rows:** a layer row reads Looks good once every one of its files has a matching look, including ticks made by hand.
- **Looks good (footer):** marks every file of the view shown (a layer, the whole stack, or the plain diff) as looked. It also moves D183's bookmark (`MarkReviewed` with the shown trees), so "Since I last looked" counts from that press. Once every file shown is looked, the button reads "✓ Looks good" with a green tint, and pressing it again clears those looks. A file row's tick circle marks or clears that one file.
- **What shows:**
  - the file bar reads "✓ You looked at this";
  - the footer status reads "Layer 1 · 0 of 3 files looked at";
  - layer rows read "✓ Looks good" when every file of the layer was looked, or "● Changed since you looked" when a looked file's fingerprint no longer matches. A layer is re-checked only when its diff is read;
  - the header switch is "All changes | Since I last looked · N files", with the caption "N FILES CHANGED SINCE YOU LOOKED";
  - the end state is "✓ All 3 layers look good · 4 comments done".
- **Base menu:** Mark reviewed and "Since I last reviewed" leave the base menu (D183 reversed); the switch replaces them.
- **Per-file look:** `FileLook { Never, Looked, Changed }`. A file never looked has an empty grey circle. A looked file has a green circle with a white ✓. A file that changed since you looked has an amber circle (1.5 pt ring #d98200, fill #fff4e0).
- **Looked button (Fable):** one style in every state, with fill #e9f7ed, 1 px border #b7e0c2, and label and ✓ #28a745. Hover may only darken the fill to #dcf1e2, and press to #cfebd7.
- **Widths (Fable):**
  - At full screen, or at panel widths of 560 pt or more, the switch sits in the Diff header, and the status and Looks good sit in the footer, left of Send.
  - Narrower, the status and Looks good get their own 34 pt row above the footer, always shown. The switch gets a 36 pt row, shown only once a look exists.
  - The status never ellipsizes. It drops clauses from the right: first " · N drafts", then "Layer N · ". The end state drops " · N comments done" when the count is 0 or the full string doesn't fit.
  - A layer row's repo label shows only if its first word fits in full; otherwise it is hidden, with no lone "…". The noun agrees with the count: "1 file", "1 comment done".
- **Not yet as drawn:** the Send summary's "layers 1 and 3 look good" (no Send summary popover exists), and 4-3's "N pull requests are up to date / Open the pull requests" (no data source).
- **Reverse:** drop `ReviewState::looked` and the switch, and restore D183's menu entries.

## D853
An agent's note on a line is one card (DIFFUI2 manager, 2026-11-19; UXSIMPLE step 8 (d), frame 3-4). Provisional
- **What it is:** the card reads "<agent> [Note] line N", then the body, then these actions:
  - **Send to <session agent>:** makes it the person's own comment, on the note's anchor, and sends it. The note goes only once the send succeeds.
  - **Edit:** opens the box. Commenting from there sends the edited text the same way.
  - **Drop:** removes the note.
  - **Draft for GitHub:** only in a pull request review.

  "Make it a draft" is gone from the Diff panel's cards.
- **Counts:**
  - The footer status adds "· N notes".
  - A file row shows an orange note chip after its grey comment count.
  - A layer row shows "N drafts · M notes" in accent, right-aligned in its status slot, before any look status. It shows only for a layer whose files are known: the one shown, or one marked Looks good. Whole stack shows the total.
- **Not yet as drawn:** settled by D856: an agent's GitHub draft is the same line card, "<agent> [Draft] line N".
- **Reverse:** restore `plain_card` for agent notes in `ui/changes/src/comment_cards.rs`, and drop `CommentsAction::SendNote` and `NoteSent`.

## D854
Showing a change "with notes" is part of the show skill; there are two review skills, not three (DIFFUI3 manager, 2026-11-19; UXSIMPLE step 8 (f), UXSIMPLE_REVIEW §2.7; narrows D791). Provisional
- **What it is:** `order-humata-review` is gone. Its steps are `render-humata-review/with-notes.md`: a reading order (top-down or bottom-up), 2–6 groups and neutral notes that explain, sent in the same `review_prepare` as the layers or the narrow spec. render's description now carries "walk me through this", "explain this change", "show it with notes" and "what order should I read this in"; `review-humata-change` stays the only skill that judges.
- **Installed copies:** `daemon/machines` `own_skills` removes a skill Humata once installed and no longer ships, so agents' folders lose `order-humata-review` at the next start. No code change.
- **Docs:** RV-33 lists two skills; `skills/README.md` lists order under "Merged or dropped".
- **Reverse:** restore `skills/order-humata-review/` (SKILL.md, guide.md) from before this decision, drop `with-notes.md`, and restore the descriptions of render, review and humata-pull-requests.

## D855
Done comments always show, folded; no "Show done" control, no Hide, no extra Sent look (DIFFUI3 manager, 2026-11-19; settles D851's gaps; frames 4-2 and 4-3; Fable 5.2). Provisional
- **"Sent · the agent changed these lines": dropped.** FRAMES.md says of 4-2 that addressed and resolved collapse into Done, and 4-2 draws Addressed as "Done by claude" with Reopen. Sent stays plain. "Changed since you looked" on the layer (4-1) and the "Since I last looked" switch already tell that lines moved.
- **Archive → Hide: dropped.** No frame draws a menu on a card. `CommentsAction::Archive` stays in the reducer with no card control.
- **Show done: no control.** 4-2 and 4-3 count and fold done comments ("✓ 3 comments done on this file · Show", "4 comments done"), so a comment the person resolved must stay listed. The reducer now always lists with resolved comments (`Which::WithResolved`) and keeps a resolved comment when it changes; the per-session Show resolved flag and `CommentsAction::ShowResolved` are gone. The file fold's Show / Hide is the only show-done control. `CommentsSaved` still parses old saved files and is ignored.
- **Tests:** `features/review:comments_test` (`a_comment_the_person_resolves_stays_listed_as_done_and_reopen_brings_back_a_draft`, `comments_for_a_file_sit_under_their_last_line_with_resolved_and_without_archived`, the opening listing takes resolved ones). `ui/changes` `a_file_folds_only_when_every_comment_is_done` already covers Addressed plus Resolved.
- **Reverse:** restore `SessionComments::show_resolved`, `ShowResolved`, `which()` and the resolved filters in `features/review/src/comments.rs`.

## D856
An agent's GitHub draft is the one agent card (DIFFUI3 manager, 2026-11-19; settles D853's gap; UXSIMPLE 3-1, 3-4; Fable 5.2, under its golden authority). Provisional
- **What it is:** outside the Submit sheet, a pull request draft an agent wrote reads "<agent> [Draft] line N" (the agent's command, as 3-4 and 4-2 write it), then the body, then Edit and a faded Drop as accent text links with a 16 pt gap, as 3-4 lays out its actions. The chip is 3-1's blue draft badge (#0a6cf0 on #eef4ff, border #c9d8f5; the draft chip's own tones in dark). The card has the plain grey line; the accent outline is for the open box only. No Send, no Draft for GitHub, no bold primary: the footer's Submit review… names where it goes.
- **Unchanged:** the person's own GitHub drafts and detached drafts keep their cards.
- **Goldens:** native 68, 68b, 68c and 69 re-recorded; design 68 changes for this card only, and design 69 with it, since its backdrop is 68. No uxsimple frame draws a pull request session, so this extrapolates from 3-4's "one agent-authored card everywhere"; design 68 goes to the owner in the next small render batch.
- **Tests:** `desktop:native_test` `pr_session` (Edit and Drop on the agent's draft work by id).
- **Reverse:** drop `agent_github_draft` and `is_agent_github_draft` in `ui/changes/src/comment_cards.rs` and `agent_draft` in `desktop/tests/design/pr_review.rs`, then re-record 68 and 69.

## D857
⌘T, and New agent tab's opening prompt and "not installed" (DIFFUI3 manager, 2026-11-19; settles D849's gaps; UXSIMPLE 3-3, 3-4; Fable 5.2, under its design authority). Provisional
- **⌘T (built):** whenever a session is open, ⌘T opens a Terminal tab in its focused tab group, as the strip's `+` with Terminal does. A full-screen panel gives way first, as a menu pick does. With no session open it does nothing. It never sets "last used", which is for agents only. The hint shows dimmed and right-aligned on New agent tab's Terminal row, as 3-3 draws it. The ⌘/ sheet lists "New terminal tab ⌘T" after New session, and the palette offers it in a session.
- **Opening prompt (ruled, not built):** an agent picked from New agent tab, and never Terminal, ⌘T or the strip's `+`, gets one prompt pasted and submitted. The wording is fixed at pick time:
  - a stack layer: "Look at layer N of the stack in the Diff panel. Don't edit files. Leave what you find on the lines."
  - the whole stack: "Look at the whole stack in the Diff panel. …"
  - a plain diff: "Look at the changes in the Diff panel. …"
  - a pull request: "Look at pull request #N in the Diff panel. …"
  It is sent once the agent has painted its first screen and gone quiet, waiting on events with a 15 s deadline. If the person types first, or the deadline passes, nothing is sent and nothing is left half-typed.
- **"not installed" (ruled, not built):** the menu lists only the agents Settings has switched on, in Settings order, installed ones first. A missing agent shows a grey dot, dimmed, as "codex · not installed". It isn't clickable, has no hover and never gets "last used". While presence is unknown, a row draws as installed. The daemon pushes CLI presence per machine with the machine's state; the menu never polls for it. This needs daemon, proto, live client and fake wiring.
- **Goldens:** native shortcuts-sheet-dark and shortcuts-sheet-light re-recorded.
- **Tests:** `features/session:tabs_test` (a new Terminal tab, and nothing without a session), `features/app:every_action_test` (⌘T is in the palette with its shortcut), `desktop:native_test` `diff_full_screen::cmd_t_opens_a_terminal_tab_and_leaves_full_screen`.
- **Reverse:** drop `SessionAction::NewTerminalTab` with its app arm, `Command::NewTerminalTab`, `keys::NewTerminalTab` with its binding, SHORTCUTS row and shell handler, and the Terminal row's hint, then re-record the shortcuts sheets.

## D858
New agent tab opens a fresh agent, done comments fold only on a ticked file, and the REVIEWER exit flow (DIFFUI4 manager, 2026-11-19; UXSIMPLE 3-3, 3-4, 4-2, 4-3; Fable 5.2, under its design authority). Provisional
- **Fold keyed on the tick (4-2/4-3):** a file's done comment cards fold into one line only once the person ticked the file Looks good. An unticked file keeps its done cards open with Reopen, as 4-2 draws. Native frames re-recorded: review-done-by-agent, review-done-by-agent-dark, review-done-by-agent-full, review-done-folded. Fable approved all four.
- **Always a fresh tab:** an agent picked from New agent tab never attaches tab 0 (the agent that wrote the change) or an earlier tab. Picking the same agent twice gives two tabs. `OpenTerminal`'s attach stays for its other callers.
- **How it starts:** today's daemon refuses a second agent and can only open shell tabs. So `SessionAction::OpenAgentTab` opens a shell tab in the session folder and runs these steps:
  - waits for its first paint and quiet;
  - types the agent's command and Enter;
  - waits for the agent's own paint and quiet;
  - types the D857 prompt.

  Details:
  - The shell's echo of the command does not count as the agent's paint. Only output after the echo's line end does, or a switch to the alternate screen.
  - One 15 s deadline covers the whole sequence, measured on the clients' clock against terminal link frames.
  - The person typing in that tab cancels the rest.
  - Typing uses TerminalService.Type through `Daemon::send_to_tab`.
  - The prompt belongs to its own open (`AgentTabOpened`), so another tab opening meanwhile never takes it.
- **Label:** the tab shows the bare agent command with the agent dot (3-4: `copilot ×`). For now the app keeps it (`SessionState::agent_tabs`, keyed by session and tab index; the daemon never reuses an index). It is re-applied when the daemon sends the session again. It does not survive an app restart. Follow-up for main: the daemon records a shell tab's label and agent kind.
- **REVIEWER exit mapped onto D849:**
  - open: a pick leaves full screen and selects the new tab;
  - hide: select another tab or go back to the full-screen Diff, and it keeps running;
  - show: click its tab;
  - end: its ×, then End for good;
  - restart: after a daemon reboot the tab is listed again.

  The dock, the Show/Hide review terminals button and a reviewer session kind no longer apply.
- **Tests:**
  - `features/session:opening_prompt_test`: command then prompt; the echo isn't paint; the person typing first, or after the command; the deadline; a tab opening meanwhile; the label; Terminal and ⌘T never type.
  - `features/review:opening_prompt_test`: the four wordings.
  - `features/app:agent_tab_label_test`.
  - `desktop:native_test` `diff_full_screen::an_agent_pick_waits_with_its_prompt_and_terminal_has_none`.
  - `desktop:live_flows_test` `reviewer::a_reviewer_tab_opens_hides_shows_comes_back_after_a_reboot_and_ends_on_its_cross`, against a real humatad2.
- **Reverse:**
  - Send `OpenTerminal{agent}` from agent_tab.rs again.
  - Drop `OpenAgentTab`, `AgentTabOpened`, `PersonTyped`, `OpeningPromptDone`, `opening_prompt.rs` in features/session and features/review, `agent_tabs` with its events.rs line, and `Daemon::send_to_tab`.
  - Drop the `.filter(|_| ticked)` in diff_body.rs and re-record the four frames.

## D859
Goldens 42 and 43 are redrawn for Back and the Sent chip (Fable 5.2; D848, D851, D855).
- Both frames' headers lead with the 24 pt Back ghost, as the app draws it.
- The sent card on line 7 reads "You · line 7 [Sent]", with the same chip and header row as the Draft card.
- **Reverse:** drop the `back` flag from `desk_header_back` and `session::sent_comment`, then re-record 42 and 43.

## D860
Golden 12 draws one title bar in full screen (Fable 5.2; DS-14, DV-18, D848).
- One 52 pt bar holds "Board · payments-rewrite", with Exit full screen and the 16 pt Close where the app puts them. The panel's 36 pt header row is gone.
- The full-screen Close is the app's 16 pt ghost in every full-screen frame, so 11 and the other full-screen frames move a little too.
- **Reverse:** draw `panel_header` in `board_panel` again when `full`, drop the Board title extra in `session::panel`, and re-record.

## D861
Golden 68 is redrawn for the review footer, the file bar and Back (Fable 5.2; D848, D850, D856, DS-2, DS-53, DS-55, RV-1).
- The header leads with Back, and there is no session-bar Submit while the Diff shows.
- The file bar reads "acme/api · refund.rs src/ · 1 of 2 ‹ › ↗".
- The footer reads "4 drafts · 3 by claude, 1 by you", with "Submit review…" as its one primary.
- The footer count names the agent by its command (`Agent::command`) and is never cut. If it can't fit beside the buttons, it falls back to the bare "4 drafts" (`review_header::drafts_count`).
- The agent's draft card keeps D856's plain grey line. The accent outline branch in `comment_cards.rs` is gone, so only the open box is outlined.
- Review file rows lead with the file glyph, with the letter mark after the name.
- **Reverse:** take the fit check out of `drafts_line`, hard-code the agent name again, and re-record 68.

## D862
Golden 68 loses the inline Undo and gains the D283 strip, which is built for 0.4.0 (Fable 5.2; D59, D283, PR-22).
- Humata draws nothing inside the agent's terminal output, so the "Undo" after `review_note refund.rs:140` is gone from golden 68 and from SPEC's sketch of the review session.
- The strip is one 32 pt row at the foot of the chat tab, above the "2 new commits" banner. It lists the agent's view changes newest first, 3 shown plus "+N", each with a ghost Undo. Frame 68 draws "Noted refund.rs:140 · Undo" and "Opened refund.rs:140 · Undo".
- A change caused by the code moving shows without Undo. The strip is absent when there are no changes.
- Undo sends the existing `FollowerAction::Undo(undo_id)`. Only the view is new.
- It ships in 0.4.0 and does not hold 0.3.7. Until then native/68 differs from the golden by the strip alone.
- Golden 68 and 69 draw the `pr-reviews` rollup green, as native does, and goldens 190 and 191 gain the session header's Back (D848).
- **Reverse:** drop the strip view from the chat tab and re-record 68 and 69 without it.

## D863
Shortened text keeps its whole text, and every full-screen arrow names its key. The accessibility tree changes to match (Fable 5.2, REVIEWUI batch 3; DS-17, DS-46, DS-57, DS-79, DS-80).
- Through `full_text`, these elements get a hover and a label: the session title, the full-screen title line, the file row (`repo/path`), the file note, the diff-base and per-repo base chips, the repo error name, and the PR review's repo name. The repo name now ellipsizes after the branch (case 05 step 10).
- Tooltips and descriptions:
  - Exit full screen: "Exit full screen  ⌃⌘↩".
  - The terminal arrow: "Expand terminal  ⇧⌘↩" or "Return to columns  ⇧⌘↩".
- A force-push with new commits reads "The PR branch was force-pushed with N new commits" (case 12). The PR notice bar is a `Status` with its words as its label.
- Native view snapshots gain the `Label`, `description` and `Status` lines. PNGs must not move; any PNG diff goes back to Fable.
- **Reverse:** drop the `full_text` wrappers and the tooltips, then re-record the view snapshots.

## D864
Claude Code's auto-mode offer stops the input gate and nothing else (AGENTS2 minion, pick by Fable 5.2, 2026-11-19; amends D845, D762).
- Live check O1 recorded Claude Code 2.1.328 drawing "Teach auto mode about your environment?" (1. Yes, 2. Not now, 3. Don't show again) after a turn, with no hook. A send typed then would go into it and its Enter would pick Yes.
- `ScreenQuestion::Offer` holds those exact words in `runtime/src/screen_question.rs`. The input gate refuses Humata's text while it shows (FailedPrecondition: dismiss it in the terminal, then type again), and a held first message waits. Humata never answers it.
- It holds no Needs you: `screen_ask` ignores an offer (`ScreenQuestion::needs_you`), because the agent isn't waiting on the person.
- D845's freeze is narrowed, not lifted: a new gate pattern needs a recorded live screen and no hook, and only refuses.
- **Reverse:** drop the `Offer` row if Claude Code fires a hook for the offer, removes it, or rewords it.

## D865
The real Diff panel's stack view copies frames 119–121 (STACKVIEW minion; picks by Fable 5.2, 2026-11-19; round 1 rejected three goldens, round 2 approved all eight).
- Stack header: "STACK · N LAYERS". The picked layer shows its drift strip; Whole never does. The drift strip's "Ask the agent to send it again" is a quiet bordered button.
- The Whole stack row keeps the file count from its last Whole read while a layer is picked (`Fetch.whole_files`). Its repos include every repo in the stack's Whole specs, so untouched repos stay listed when a layer read replaces the diffs.
- Footers:
  - A layer steps to the layers beside it, and offers Looks good.
  - Whole always shows one link, "N pull requests open" or "Push & PR…" when there are none. Both open the push menu.
- In a stack:
  - file rows and repo heads are 24 pt;
  - the look tick is 13 pt with a line-colour border;
  - the list caps at 240 pt;
  - the key chip moves from the file header into the file row.
- Repositories with the same short name show their full path, in the list and in the file header (`stack_repo_name`). A repo with no files draws no chevron and no totals.
- A skim group names up to 3 files (`SKIM_NAMES`).
- Deferred:
  - the two-column old/new gutter with in-line word highlight (shared component);
  - the full-screen uxs-2-2 layout (left column, layer stepper with key hints, "Layer N · x of y files looked at");
  - in a tight header, shrink the directory before the repo name;
  - count empty repos in "N repos";
  - a dim "no changes" on an empty repo row;
  - truncate, don't drop, the directory when count chips are shown;
  - a shorter base label.
- **Reverse:** revert panel_stack.rs and the stacked branches in panel_changes, then re-record the stack-* and looks-good-* goldens.

## D866
Migrate the holders shows how many terminals moved (PROGRESS T6, frame 125; minion LIVEHOLDER; pick by Fable 5.2, 2026-11-19, approved with changes, all applied).
- The daemon counts a round of moves: `HoldersMoved { done, total, started_unix_ms }` on `MigratePreview.moved` and `HoldersReport.moved`. A terminal counts as handled once it moved, or once its holder proved too old to move or out of reach, so the count never sticks. A round that stops on a refused or lost move keeps its count.
- Migrate card while it moves:
  - Title, then "From Humata · migrating…", body, the list, then the progress line where the Migrate button was. There is no button and no Cancel, because a one-at-a-time in-place move has nothing safe to cancel.
  - Progress line: "Moving <session moving now>" on the left (or "Moving terminals" when no row is moving), and "3 of 7 terminals moved" on the right.
  - List header: "Still to move · N sessions". Row notes are "moving now" and "next".
- Machine page: the same count sits in a row under the upgrade banner, titled "Moving terminals".
- Fable noted that frame 83 (the waiting card) still speaks of restarts. That is right for holders too old to move, but a card whose holders all move in place should not say so. This is a follow-up for frame 83's copy and is not changed here.
- **Reverse:** drop `moved` from the protos and `MigrateCard`, remove `moving_note`/`waiting_header`/`moved_progress` in request_card.rs and `holders_moving_row` in machine_page.rs, and delete frame 125.

## D867
Whole stack marks where each layer starts (owner's mark 2026-11-19; minion DIFFUI; pick by Fable 5.2, 2026-11-19).
- Inside each repository group of Whole stack, a quiet caption row comes before each layer's files: the layer list's small outlined number circle at the file tick's indent, then the layer title in secondary grey at the directory text's size, regular weight, no counts, about three quarters of a file row high. A click on a caption picks that layer, like its row in the layer list.
- Layers run lowest first; within a layer, files are key-first. A file several layers list sits once, under the lowest, with a grey "also N" after its directory. Files no layer lists come last under the caption "Other changes" (no circle). A layer with no files in a repository has no caption there. The skim row stays one folded row per repository at the group's end.
- When no layer carries a file list, Whole stack shows no captions and keeps today's order.
- **Reverse:** remove the captions and restore repository-wide key-first order.

## D868
The Diff header names the base once (owner's mark on the per-repo chevron, revisits D739/D510; pick by Fable 5.2, 2026-11-19).
- With two or more repositories the header reads "Diff", then "N repos" in secondary grey where "against" sat, then the All changes / Since I last looked switch; there is no header base chip. Each repository row's menu holds the header menu's entries (branches, Uncommitted only, another session's branch) and ends with "Use for every repo"; the row states its choice in words ("against main", "uncommitted only", "against <session>'s branch").
- With one repository the header keeps "against [base]" as today and the repository head shows name and counts only, so "against X" never shows twice. A stack is unchanged.
- **Reverse:** restore the header chip as a set-all control (and, for one repo, move the base to the row with "1 repo" in the header).

## D869
RV-9: the inline PR review's repository chevron does something (pick by Fable 5.2, 2026-11-19).
- A folded repository row keeps "›" and the whole row is the click target: a click picks that repository's first file, which opens it and folds the other. The open repository row has no chevron, no hover fill and no click; it keeps an empty slot where the chevron was, so its repo icon and name line up with the folded rows (Fable, 2026-11-19 frame review).
- **Reverse:** drop the chevrons entirely, or add a real per-repo fold state.

## D870: PR review requests reach the To-do list gently, through the host's gh (BOARDSIMPLE manager, 2026-11-19). Provisional

- The host fetches `is:open is:pr review-requested:@me` through `gh api graphql` at most once per 10 minutes, started in the background by a To-do list or Do next read; a failed fetch closes nothing and waits the same 10 minutes. Test daemons (admin builds) never run the real gh unless the rig sets `HUMATA2_GH`.
- One item per stack: PRs in one repository whose base is another's head. The item title is "Review: <bottom PR's title>" as in frame b7-1. An item whose PRs leave the result closes itself; a re-request reopens it. Pin and Hide until survive every sync.
- **Reverse:** drop `with_review_requests` from `wire_board.rs`.

## D871: Agent inbox asks carry their case, and an agent can be asked why (INBOXHOOKS manager, 2026-11-19). Provisional

- An agent's MoveSession with an empty reason is refused with a fix telling it to say why.
- A session move asks through the same gate as other asks. Its card shows From/To (channel · title), what it carries, the terminals that move along and keep running, and warns about forks staying and clones needed (frame 202).
- A file move or copy that replaces files lists those files with a note ("same content" or "the one there is newer"). The note comes from size and mtime only, with no hash. A move says it is a move. The Undo text tells the truth: there is no Undo action, and replaced files wait 30 days in the destination's trash.
- **Ask why:** `InboxService.AskWhy` (user only) marks a pending agent card. The agent's open or next `WaitResponse` returns PENDING with `asked_why=true` until the agent replies through `ExplainAsk` (1–1000 chars). The card stays pending throughout, and the reply shows in a "You asked why" panel.
- **Agent access:** Agent access settings are grouped Allow/Ask/Never rows, each row writing several capabilities. A row whose rules disagree reads Mixed. Rules no row covers sit behind "Show Other rules".
- **Reverse:** drop `asked_why` from `WaitResponse`, drop the AskWhy/ExplainAsk RPCs, and allow an empty reason on MoveSession. Requiring `reason` on every asking MCP tool was tried and reverted: it refused reads such as `board_items_list` and `file_read`, so it needs a narrower list of tools that ask.
- Design frames 26/27, the old two-pane Inbox, are retired: 200–204 replace them.
- Batch 3 (2026-11-19):
  - An asking MCP tool whose schema requires `reason` refuses an empty or blank one in one central check; reads keep an optional reason.
  - `inbox_explain` answers the owner's Ask why, and `inbox_wait` says `asked_why` with `next: inbox_explain`. Forwarding AskWhy/ExplainAsk to the host is only listed in svc_away's registry; the host-side replay in svc_board is not wired.
  - Frame 209's notification is built as content only (`humata2_inbox::notification`, never for to-do asks, no Yes). Posting an OS notification waits for NOTIFY (0.6.1).
  - Settings › Agent access shows the same rows as a channel, over the default rules (kept under `ChannelId::global_rules()` in the client; live reads/writes go through Get/SetDefaultRules).
- Batch 4 (2026-11-19):
  - Host-side forwarding of `AskWhy` and `ExplainAsk`: `svc_away::Forwarder::call_as_user` carries `Principal::User(MachineUser { outside_sessions: true })`; `svc_board` replays `AskWhy` as `Caller::User(UserVia::Host)` (refusing agents and in-session users) and `ExplainAsk` for sessions placed on the forwarding machine; `svc_inbox` routes missing `AskWhy`/`ExplainAsk` up to the host and allows `Caller::User(UserVia::Host)` on `AskWhy`.
  - File-read asks (`Call::ReadFiles`) now build `FileAccessPreview` with `From` (resolved session title/agent or channel name), `Files`, `Access`, calm `affects`, and `undo_text`.
  - IN-8 count formatting lives in `humata2_inbox::today_summary` (omitting the rule-followed clause when `0`), wired end-to-end through `ListRequestsResponse.allowed_without_asking_today` and `Guard::allowed_without_asking_today`.

## D872: Session panels show the To-do list, Desk sessions too (BOARDSIMPLE manager, 2026-11-19). Provisional

- The sidebar row, the panel title, the rail label, the full-screen title, the View menu and ⌘3 all say "To-do" and open the To-do list. Beside the terminal, while an item is open in the drawer, the panel header is the way back ("‹ To-do", then the key · channel), as frame 191 draws it. In full screen that row leads the body.
- A Desk session's Board panel no longer keeps its own lane board with a scope: it shows the Desk session's slice of the To-do list like any other session (frame 37).
- The fake list ties items to the fixture sessions (PAY-3, PAY-7 and PAY-9 to Ledger; A-1 to A-3 to Token refresh retry; TD-3 and TD-4 to the Desk's Plan Q4), so the panel goldens show items and frame 191's drawer.
- The old board page stays reachable only through the palette's "Go to Board" until its code is removed in one change with its tests.
- After Fable's re-judge, the session panel's drawer follows frame 191: no key line, channel, sessions, waits-on or description rows; one 74 pt label column on the ask card with wrapping values; Due is the date field plus a word ("tomorrow") with the shortcuts on a second line; Impact a row under Effort; "ticked by claude" right of STEPS and "Add a step…" under the steps (page and panel); the History · Pin · Hide until… footer pinned to the panel's bottom. Frame 12 is laid out while Ledger works, like 07 and design 12. The panel's accessible name and the channel tab say "To-do" (tab id stays `channel-tab-Board`).
- The live test that the session panel shows the channel's board with the session's items first is removed: the panel shows the To-do list now (panels_test covers it).
- **Reverse:** restore the `desk_board` branch in `ui/src/panels.rs` and the `Route::Board` targets.

## D873: The Fork dialog opens collapsed on Code + context (DIFFUI manager, ruled by Fable, 2026-11-19). Provisional
- The Fork dialog opens collapsed on Code + context: the title, one sentence stating the selected kind's consequence, one muted facts line ("Agent · machine · branch <name>", plus "· before turn N" when forking from a turn), and a footer with a quiet "More options" chevron button at the left and Cancel / Fork at the right. The Preview block and the "Same agent" check line are removed; the different-agent summary notice stays, shown only when the agent differs. More options is a per-dialog disclosure, closed on every open and never remembered, that reveals in order the three kind cards (replacing the sentence), Fork from, Agent, Machine and Branch with its hint, under the existing per-kind hiding rules; the facts line stays and updates live. The dialog opens expanded only when the caller presets a kind other than Code + context or an agent or machine different from the parent; a turn preset alone stays collapsed. Width is unchanged, height hugs the content up to the existing maximum and then scrolls. Return forks and Esc cancels as before, with initial focus on Fork. Goldens for frame 62 are re-recorded in collapsed and expanded states, light and dark.
- Amends D102, D188 (UXSIMPLE review: "Fork opens on Code + context with the other kinds and fields under More options").
- **Reverse:** render the dialog always expanded and restore the Preview block and check line (the D102/D188 layout), then re-record the previous goldens.

## D874: The board page and its crate are gone; To-do is the one list (BOARDSIMPLE manager, 2026-11-19). Provisional

- `Route::Board`, the `humata2_board` crate (kanban, table, dashboard, drawer, the Scope picker), the palette's "Go to Board" and "#channel board", the Desk's lanes and the unused board panel are deleted with their tests. Finishes D872's "until its code is removed".
- **Saved state from earlier builds:** a saved board route, in the route or the trail, opens the To-do list; a saved "Go to Board" palette pick is dropped; the old `board` view key is ignored. Nothing fails to restore.
- **Channel page:** its To-do tab (id still `channel-tab-Board`) is the To-do list narrowed to that channel. The empty-channel hint says "The To-do list is empty too: add items from the To-do tab, or ask an agent to."
- **Agent views:** a board view (a channel's or the whole board) opens the To-do list with the matching channel filter; a session's board view opens it narrowed to that session. Its Showing bar leads the To-do page, and Revert puts the person's own filter back. The view tab calls these "To-do".
- **Pins:** a pinned board item opens on the To-do list with its drawer (the item's key is the board item's id).
- A channel's narrowing stays on the To-do list until the person picks All, like any other filter.
- Kept: session VIP (NV-23), Inbox VIP Migrate cards, and the board item's `vip` field the migration reads (D833, D837). The daemon's board service and the agent capabilities stay; only the desktop's board went.
- Frames 21-24 of the board, their look_test entries and design module are removed; frame 122 draws the To-do list behind the New session dialog.
- **Reverse:** restore `features/board`, `ui/workspace/src/board.rs`, `panel_board.rs`, `ui/desk/src/desk_board.rs` and the route from git.

## D875: Deleting many sessions: select mode and one confirming sheet (MASSDELETE manager, Fable 5.2 design, 2026-11-19). Provisional

Frames are in `workflow/milestones/mockups/massdelete/` (m1-1 to m1-4, m2-1 to m2-6; see FRAMES.md there).
- **Entering:** ⌘-click a session row in the sidebar, on Home or on a machine page; Home's toolbar "Select"; the "Select" link beside a machine page's "Sessions on it · N"; context menu "Select sessions…"; palette "Select sessions…". The sidebar has no permanent control. Select mode belongs to the list it was entered in, and leaving the page exits it. **Reverse:** add a Select control to the Channels header, or make the selection app-wide.
- **Ticks:** golden checkboxes, 13 pt in the sidebar's left gutter and 14 pt before the status dot on Home and the machine page. Ticked rows are faintly tinted. In select mode a row click toggles; row buttons are hidden and New session is dimmed. **Reverse:** keep row buttons live.
- **Whole channel or machine:**
  - In the sidebar, a channel's fold chevron becomes a tri-state box, and so does each Hosts row. Offline machines and empty channels show a dashed box that can't be ticked.
  - On pages, the bar's "Select" menu offers "Every session in a channel", "Every session on a machine", "Everything on Home ⌘A" and "Nothing".
  - Desktop picks are live sessions only; archived sessions are deleted only through "Delete all sessions…".
  - **Reverse:** include archived in channel or machine picks.
- **Context forks:** they are ticked automatically with a paler tick that can't be cleared, and they count ("4 selected · 3 picked · 1 fork goes along"). The sheet title uses the plan's count. **Reverse:** add them only in the sheet.
- **Action bar:**
  - In the sidebar, a card above Settings: "N selected", a note, Cancel, and an outlined red "Delete N sessions…".
  - On pages, a 48 pt bottom bar with the count, the note, the Select menu, key hints, Cancel and Delete.
  - Keys: ⌘A, Shift-click range, Space, arrows, Return or ⌘⌫ opens the sheet, Escape exits. Cancelling the sheet keeps the ticks.
- **Entries elsewhere:**
  - Palette "Delete all sessions…" appears in red only when the query matches, ranked below other matches, with no shortcut.
  - Settings > Storage gets a "Sessions" card with the "Delete all sessions…" danger button.
- **Sheet:** 700 wide, in the delete-channel style.
  - Order: title with the count ("Delete 4 sessions?", "Delete all 23 sessions?"); the scope line; a totals block outside the scroll area (terminals that end, never-pushed work, forks, unreachable machines); the list grouped by channel; the grey trash box; the count field (all only); the footer with a hint, Cancel and Delete.
  - Each row names each terminal that ends with its state, and its unpushed work, with a "Delete that work too" box per session.
  - A context fork going along is indented under "Goes with <parent>"; forks that stay get one line.
  - In delete-all, sessions with nothing to lose fold to one line per channel. The list scrolls and the sheet never passes the window.
- **Delete all:**
  - Type the number that will be deleted; unreachable sessions are not counted.
  - The button is a filled red "Delete all N sessions", the app's only filled red, live only when the count matches and every work box is ticked.
  - A picked set uses the outlined danger button.
- **Stale plan (NV-27):** the sheet stays open with an amber notice: "Something changed since this plan. Nothing was deleted.", one line per change, then "This is the new plan. Check it and delete again." Changed rows are tagged "changed". Only those rows' work boxes clear, and the count field clears if the count changed.
- **Unreachable machines:** one totals line plus a last group "<machine> · can't reach, left alone · N sessions". If nothing is reachable the button stays off.
- **After:** a toast, "Deleted N sessions. They are in the trash on {machines} for 30 days.", with no action. The sheet and select mode close, and forks that stayed move up.
- The strings are in Fable's report, which is copied into the MASSDELETE lane. Workers copy the frames exactly.
- **Reverse:** remove the select mode and the sheet; the CLI stays.

## D876: Bulk session delete in the daemon, CLI and agent access (MASSDELETE manager, 2026-11-19). Provisional

- **One path:**
  - `PlanDeleteSessions`/`DeleteSessions` take a scope (picked ids, a channel, a machine, or all), give one token (10 min, one use) and do one delete.
  - The old one-id `PlanDelete`/`DeleteSession`, never built until now, call the same code with one id and keep their gates (OwnSessionEnd, SessionDeleteOther).
  - A channel or machine scope includes archived sessions. The desktop sends its live-only picks as ids.
- **Stale plan (NV-27):** the delete re-lists. Any new or gone session, terminal, unpushed work or fork answers `changed` with the new plan and the changes in words, and touches nothing. A terminal's state change alone (working to idle) is not a change.
- **Per session:** intent note, end terminals, withdraw inbox requests, end grants, the record exported to a side file in the folder, the folder to the machine's trash (`was` = Session), then the record removed. A restart finishes a cut delete. Restoring from the trash brings the session back archived, with its folder and work.
- **Forks and Desk:** context-only forks go with their parent (D187); code forks stay. Desk sessions are never in a bulk scope; Clean up deletes them (D306).
- **Machines:** offline machines are listed as left alone and not touched. Every reachable machine passes a check before any machine deletes. A machine that fails after the check leaves its sessions, and the error names what went.
- **Unpushed work:** it blocks unless the request says to delete it (per session in the sheet, `--delete-work` in the CLI). Scope all needs the typed count to equal the number that will be deleted, and a present user.
- **CLI:** `humata2 sessions delete [<id>...] | --channel | --machine | --all`, plus `--delete-work` and `--count N`.
  - `--yes` never skips the count for `--all`.
  - `--dry-run` prints the plan.
  - A stale plan prints the changes and exits non-zero.
- **Agents:**
  - New capability `sessions_delete_many` (74), Never by default, needing the host. It has an Agent access row "Delete many sessions at once" under FILES AND SESSIONS.
  - An agent never gets scope all, and never its own session.
  - A plan that ends running terminals always goes to the owner's Agent inbox, even under Allow (PER-32).
  - The MCP tool `sessions_delete_many` is listed only while the caller's rule is not Never (`notifications/tools/list_changed` when that flips).
- **Reverse:** drop the two RPCs and the capability; the one-id RPCs go back to not built.

## D877: Inline PR review virtualization box height and per-file diff errors (DIFFVIEW Gemini lane, 2026-11-19). Provisional

- **PR review virtual box (DS-29):** inline PR review files above 1,500 lines (`REVIEW_VIRTUAL_FROM`) render in a virtual `gpui_kit::list` box of height `((viewport_height - 104) * 0.6).clamp(160, 720)` with no invented border (matching frame 68's code block styling). Files at or under 1,500 lines keep inline blocks unchanged.
- **Per-file errors in `clients/live_diff` (DS-36):** after `GetDiff` succeeds for a repository, a file-scoped `GetFileDiff` failure (`Refused`, `NotFound`, `Conflict`) records `FileDiff.error` and clears that file's lines so the rest of the repository still loads; transport/version/config errors (`Offline`, `NoDefaultBranch`, `MigrationNeeded`, `NotServedYet`, `Scripted`) still abort the read.
- **Reverse:** restore unbounded inline rendering in `review_code` and restore `?` propagation on `get_file_diff` in `clients/live_diff/src/diff.rs`.

## D878: Extra terminal tabs persist label and agent on the daemon, and OpenAgentTab launches agents directly (REVIEWER Gemini lane, 2026-11-19; D858 items 1–2). Provisional

- `Session.shell_tabs` (`repeated ShellTab { tab, label, agent }`) and `Tab.agent` carry every extra terminal tab's persisted label (`TabRecord.title`) and agent kind (`tab_agent:<tab_id>` in the daemon store), so agent-tab labels, dots, and counts survive daemon and app restarts without client-side memory.
- `TerminalService.OpenAgentTab { idempotency_key, session, agent, prompt } -> { tab, index }` resolves a non-shell agent via `Agents`, spawns it directly under its own holder with a fresh conversation id in the session folder, persists its label and agent kind, notifies session subscribers, and delivers `prompt` in the background once the agent reads keys. Plain shells are refused with `InvalidArgument` (`OpenShellTab` stays for shells).
- Extra agent tabs stay `TabKind::Shell` in `tabs` so they sit after tab 0, remain closable via `CloseTab` / `End for good`, and never hijack session-wide `typing_tab`.
- **Reverse:** remove `OpenAgentTab` and `Tab.agent` from `terminal.proto`, remove `ShellTab` / `shell_tabs` from `sessions.proto`, drop `svc_terminal/src/agent_tabs.rs`, and restore `AppAction::Session(SessionAction::OpenAgentTab)` pass-through to client-side shell typing.

## D879: Visual review and design rulings for 0.3.10 batch (judged by Gemini, 2026-11-19). Provisional

- **Changed goldens reviewed (judged by Gemini):** PASS on `native/diff-stress-pr-huge-file`, `native/fork-dialog{,-dark,-expanded,-expanded-dark}`, `native/01-session-working-light`, `native/11-panel-full-screen-diff`, `panels/changes-light`, `native/35-empty-channel`, `design/122-new-session-cloning`, `design/206-session-after-yes`, `design/209-agent-inbox-notification-dark`, `native/26-inbox`, `native/25-home`, and `native/72-stop-confirm`. Fixed `design/35-empty-channel` tab label from `"Board"` to `"To-do"` in `desktop/tests/design/desk.rs`.
- **DIFFVIEW DS-29 virtualized PR box border (judged by Gemini):** keep the `> 1,500`-line PR review virtual box borderless (`look.term` fill, no 1 px hairline border), matching frame 68's inline code blocks. **Reverse:** add `.border_1().border_color(rgb(look.line))` in `ui/changes/src/panel_changes/review_header.rs`.
- **TODOLIVE / BOARDSIMPLE New session sheet (judged by Gemini):** keep `ui/overlays/src/overlay_new_session.rs` on the approved frame 31 layout (`31-new-session`) for 0.3.10 and keep `StartNewForItem` leaving `first_message` empty (deferring any `b4-3` sheet redraw to a post-0.3.10 pass). **Reverse:** add a `b4-3` mode to `overlay_new_session.rs`, prefill `first_message` in `features/app/src/new_session.rs`, and re-record `31` and `122`.
- **D868 design frames (judged by Gemini):** design frames `01`, `02`, `11`, and `84–86` should be updated to match D868 (`"2 repos"` header count and per-repo `"against <base>"` chips) in a follow-up design-fixture pass. **Reverse:** keep D739/pre-D868 design fixtures.

## D880: MASSDELETE T10/T13 visual review and 0.3.11 closure (judged by Gemini, 2026-11-19). Provisional

- All seven re-recorded `native/massdelete-*` snapshots (`home-select`, `machine-select-all`, `plan-picked`, `plan-all-empty`, `plan-all-typed-dark`, `plan-stale`, and `after-delete`) PASS visual review against `mockups/massdelete/` after fixing bottom-bar ellipsis truncation, dark-mode count-input contrast/focus and list bottom-border/scroll coverage, dark-pill `×` toast styling, and `FakeDaemon` context-fork/stale-line/inbox-withdrawal parity (judged by Gemini).
- **Reverse:** revert the `select_mode.rs`, `overlay_delete_sessions.rs`, `overlay_toasts.rs`, and `clients/src/daemon/fake/mass_delete.rs` polish changes.

## D881: FUSEFS makes every machine feel like one file namespace over one `BLAKE3` manifest/chunk data plane, without blocking remote sessions on host sync (FUSEFS spec, 2026-11-19). Provisional

- The owner-level outcome is one machine: an agent addresses code, checkouts, context and another session by logical Humata path. It does not choose SSH, upload/download or a physical host path on the success path. Machine placement remains an inspection view.
- One authenticated peer protocol (`fusefs_cas::ChunkTransport`) serves FUSE `read`, owner copy/upload/download, materialization, config-bundle sync, checkout and `SessionSnapshot` move payloads. Files have paged manifests and 256 KiB logical chunks batched in at-most-1 MiB replies, verified and cached in one machine-wide sharded store by decoded 256-bit `BLAKE3` digest (`T1` compares fixed 256 KiB vs FastCDC for snapshot dedup). A per-chunk planner selects negotiated lz4/zstd/none from file/content class, measured ratio, source CPU load, peer bandwidth pressure and latency budget; elevated source CPU load immediately sheds `zstd` to `lz4`/`none` so host reads never starve remote builds.
- `open` fetches no content. A read fetches one coalesced compressed chunk set and slices the result; sequential misses add bounded read-ahead. A sparse copy-on-write edit stages changed chunks out-of-band before atomic `Commit`. Ordinary remote rename/unlink are server-side metadata/trash commits with no file download.
- Remote sessions are never blocked by syncing with the local Mac (`AWAY` parity): each machine generates `s/<local-id>/context/` locally from its own transcripts, pushes invalidations into a non-blocking coalescing ring, serves outbound chunks lock-free at background QoS (`ESTALE` to remote reader on race, never blocking local writers), and skips the 250 ms revalidation wait (`0 ms` cached topology + `< 1 ms` `MachineOffline` for uncached remote peers) whenever `PeerUplink` reports `host_reachable == false`.
- Build caches and generated directories are classified at the source and exposed on remote views as synthetic stub directories containing only `.humata-omitted.json` (`user.humata.omitted_bytes`) so `rg` and `find` exit `0` with zero cache bytes transferred unless materialized by the owner.
- **Reverse:** keep FUSE reads, `humata cp`, upload/download and session carry as separate byte paths, use SHA-256 per-channel `.carry/have` stores, or let host sync/revalidation block remote sessions.

## D882: Every session has one stable `s/<id>` FUSEFS path; human trees are symlinks, and native backing is id-based (FUSEFS spec, 2026-11-19). Provisional

- `~/.humata/fuse/main/s/<session-id>` is canonical. `channels/`, `desks/` and `machines/` are generated friendly symlinks to `s/` and `m/` identities. Slugs and titles may change without moving the canonical path.
- New sessions use `<machine-root>/.sessions/<id>/{work,config}` as native backing. A same-machine rename, reparent or folder move updates metadata and aliases; the holder, `cwd`, conversation key and backing worktree stay put (`LIVEHOLDER`).
- Local `s/<local-id>/work` resolves via native symlink to `<machine-root>/.sessions/<local-id>/work` where containment permits (`1.00x` native speed), and a daemon-maintained native symlink skeleton keeps local `s/<id>` paths working when FSKit is not yet enabled or `/dev/fuse` is absent in a container.
- Cross-machine `MoveSession` keeps its lifecycle verb and preserves `carry_cheap.rs`'s Git `CloneRecipe` (local main-clone checkout + unpushed/`refs/humata/*` bundles, D313) inside `SessionSnapshot`, transferring only bundle payloads and missing dirty/config `BLAKE3` chunks while retiring per-channel `<channel>/.carry/have`.
- **Reverse:** keep channel/title folders as the physical session identity, retain every same-machine agent-state path rewrite, or drop `carry_cheap` Git bundle reconstruction during moves.

## D883: Cross-session/remote writes return `EACCES` with an Inbox request, then grant a turn/10m subtree lease with progress-bounded `fsync`; protected roots never implement delete or move (FUSEFS spec, 2026-11-19). Provisional

- A process may read every visible non-secret node across machines. Writing another session, even on the same machine, needs owner approval, except the existing channel `shared/` write on the agent's own machine. When `host_reachable == false` (`AWAY`), same-box cross-session requests journal locally (`D225`) so SSH `humata login` + `humata inbox` can approve without waking the host.
- The first unleased write creates one deduplicated Inbox request keyed to `(caller_session, dst_session, dst_parent_dir, op_class)` and returns `EACCES` (< 50 ms) with a pointer under `requests/`. It never blocks on Inbox.
- Approval grants a `WriteLease` on `<dst_parent_dir>` expiring at `min(caller_turn_end_hook, 10 minutes)`, allowing multi-syscall POSIX workflows (`sed -i` / `vim` / `Edit` random temp-sibling create + `fsync` + `rename`, `copyfile(3)`, `cp -r`) to complete under one approval while checking per-file `expected_generation` on `open`/`Commit` (`ESTALE` on concurrent remote edit).
- Remote `fsync` never returns `EAGAIN` (which POSIX tools treat as fatal write failure); it uses a 1,000 ms *progress/stall* bound, waiting to `0` while chunks actively acknowledge progress and returning `ENETUNREACH`/`EIO` (with dirty CoW extents preserved in the local journal) only if the link drops or stalls.
- Canonical session/repo/config/channel/desk/machine roots and friendly aliases return `EPERM`; delete goes through `MASSDELETE` and move goes through `MoveSession`.
- **Reverse:** bind write leases to one syscall on one exact path, return `EAGAIN` from `fsync` at 1 s wall time, or queue syscalls on human approval.

## D884: Agent config homes are native per-session directories wired to machine-local auth, with Settings-toggleable host-canonical sync, TOML/`Steel`/`bash` rewrites, and a rage-quit switch (FUSEFS spec, 2026-11-19). Provisional

- Claude, Codex and Copilot launch with `CLAUDE_CONFIG_DIR`, `CODEX_HOME` and `COPILOT_HOME` pointing to **native** `<machine-root>/.sessions/<session-id>/config/<agent>/` (never routing live agent turns, atomic temp-file renames or SQLite WAL locks through FUSE/FSKit). `~/.humata/fuse/main/s/<session-id>/config/<agent>` exposes a sanitized FUSE view with `Secret` strictly denylisted (`CredentialOutsideMount`).
- Each machine uses **its own local auth** natively (macOS Keychain where verified, or local native symlinks to `<real-home>/.codex/auth.json`, `<real-home>/.claude/.credentials.json`, etc. on headless Linux / file-auth setups so OAuth refresh works in place). Credentials and inline MCP secret literals never cross the peer link or enter `SessionSnapshot`.
- The Mac host is canonical for shared non-auth config, pushed asynchronously in the background as a `ConfigBundle` (never blocking remote session launch) and governed by `[sync]` Settings toggles in `config/config-sync.toml`: `skills = true` and `instructions = true` by default, `mcp = false` and `settings = false` opt-in, per-machine overrides, and a master `enabled` **"rage-quit" switch** that atomically reverts disabled facets back to `<real-home>/.<agent>` without mutating real home folders or restarting holders (`LIVEHOLDER`).
- Users customize cross-machine path and MCP rewrites through a 3-stage pipeline in `config-sync.toml` and `~/.humata2/scripts/`: (1) declarative TOML `[[path_rewrite]]` and `[mcp.servers.<name>]` overrides (`pass_env` for local secret binding); (2) capability-free embedded `Steel` Scheme (`steel-core`) AST transforms; and (3) target-local `bash` filter scripts (`D680`/`D688` safety rules). Post-transform checks enforce valid agent schemas, soft-skip missing 3rd-party MCP binaries with a warning, and always re-assert the target machine's protected `humata` MCP entry.
- **Reverse:** point `CLAUDE_CONFIG_DIR`/`CODEX_HOME`/`COPILOT_HOME` through FUSE (breaking machine-local file auth and exposing WAL/rename paths to FUSE restarts), hardcode all-or-nothing config sync, or omit user-programmable path/MCP transforms.

## D885: `humata2_fusefs` is a separate Rust process over standalone `//libs/fusefs/*` crates; macOS uses FSKit with `$HUMATA_FUSE_ROOT` capability roots, and Linux targets worker boxes (FUSEFS spec, 2026-11-19). Provisional

- Each machine runs `humata2_fusefs` under its user service, outside `humatad2`, so daemon `Migrate` and restarts do not disturb the mount, and `humata2_fusefs` restarts never disturb native agent `cwd`/config/SQLite descriptors.
- macOS 26.7+ uses `//libs/fusefs/fskit` (`objc2-fs-kit` + minimal signing shim if required, mounted with `MNT_DONTBROWSE` to avoid Finder drive clutter). Unless `T1` proves kernel-vouched per-request `audit_token_t` PID delivery through FSKit, the public Mac view stays read-only and launched agents receive `$HUMATA_FUSE_ROOT` (`cap_<id>` root with `..` clamped); `sessions_search`, `topology_get` and `session_info` emit `$HUMATA_FUSE_ROOT`-rooted paths so macOS agents never hit `CallerIdentityUnavailable` on tool-returned paths.
- While Linux desktop/host milestones (`LINUX`, `ANYHOST`) remain cut (`D485`), `//libs/fusefs/linux` (`fuser` over `/dev/fuse` with kernel PID binding) ships for **enrolled headless Linux worker boxes**, falling back cleanly to the native local symlink skeleton inside unprivileged containers lacking `/dev/fuse`.
- **Reverse:** embed the filesystem in `humatad2`, use `fuse-t` or kext FUSE in production, or emit public read-only paths to macOS session tools.

## D886: FUSEFS replaces file-byte MCP tools and duplicated config bootstrap only after native liveness/performance pass (FUSEFS spec, 2026-11-19). Provisional

- Remove `files_list`, `file_read`, `files_find`, `file_write`, `folder_create`, `file_fetch` and `session_read` from normal mounted agents. Keep `sessions_search` as the broad index query returning `$HUMATA_FUSE_ROOT` mount paths.
- Keep Inbox ask/wait/explain, to-do operations, session lifecycle, terminal verbs, machine operations, `humata config sync`, explicit transfer/materialization status and `PlanDeleteSessions`/`DeleteSessions` as RPC/MCP/CLI verbs.
- After stable-path and native config composition pass, retire same-machine transcript path-rewrite loops, `peer_skills.rs` direct `~/.<agent>/skills` mutation, and `seed_claude_copy`.
- **Reverse:** keep the MCP file bridge and per-machine copy bootstrap as the primary path.

## D890: The virtual filesystem and CAS engine live in standalone, domain-agnostic `//libs/fusefs/*` crates consumed by Humata via public APIs (owner, 2026-11-19). Provisional

- Following `//libs/tca-rust` and `//apps/humata2/terminal/*`, the generic virtual filesystem (`//libs/fusefs/vfs`), `BLAKE3` manifest/chunk CAS, codec planner and sparse CoW overlays (`//libs/fusefs/cas`), macOS FSKit bridge (`//libs/fusefs/fskit`), Linux `/dev/fuse` + skeleton adapter (`//libs/fusefs/linux`), and mount runtime (`//libs/fusefs/host`) are built as standalone crates under `libs/fusefs/` with **zero dependencies on `//apps/humata2/...`** (no Humata protobufs, store records, Inbox types, or agent schemas).
- Humata consumes only narrow public traits (`Volume`, `AccessHook`, `ChunkTransport`, `FilesystemBackend`) inside `//apps/humata2/daemon/fusefs_adapter` (`humata2_fusefs`) and `//apps/humata2/daemon/config_compose`, keeping Bazel compile/test caches isolated and letting all VFS/CAS/CoW/lease mechanics be tested in pure in-process isolation.
- **Reverse:** tangle FUSE/FSKit/CAS mechanics directly into `daemon/humatad` or `daemon/svc_*` crates.

## D887: Fast mode is the default; NORMALREL and FASTREL merge into one 0.5.0 release (owner, 2026-11-19). Settled

- Fast mode becomes the default for builds, runtime, and tests so the full suite runs once rather than twice (normal + fast).
- `NORMALREL` and `FASTREL` merge into a single `0.5.0` release: flip fast mode on by default, walk and audit every flow in `apps/humata2/flows/`, fix gaps, run full postsubmit (`presubmit.sh --full`), launch check, release, and install.
- **Reverse:** revert the fast-mode default back to off and restore separate normal and fast release runs.

## D888: Power-cut test deduplication and trim without losing recovery coverage (judged by Gemini, 2026-11-19). Provisional

- Audited all 17 power-cut/fault test files and trimmed strictly redundant fault points, zero-point no-op scenarios, and duplicate single-side tests while keeping every distinct write-ahead, atomic-rename, rollback, and revive recovery branch (`P1`–`P9`):
  - `power_cut_machine.rs`: dropped zero-point no-ops (`LocalFork`, `CommentSend`, `Transfer`).
  - `power_cut_both.rs`: dropped single-side duplicates (`LocalFork`, `CommentSend`, `RemoteHostVariable`).
  - `common/power_cut.rs`: added equivalence-class `keep_point()` filter (preserving `store.file.temp_written` and all three `revive.resume.{spawned,tab.put.committed,verified}` phases), added non-empty case guard, tightened `until()` poll from 50 ms to 5 ms, skipped `reports_quiet()` for `HostVariable`/`RemoteHostVariable`, and waited for running host operations + in-progress box `StartStep` in `settle()` so `Inventory::take` never races an active remote clone.
  - Deleted `power_cut_focus.rs` (10 redundant tests covered by `power_cut_host_test` + `power_cut_start_box_test`); kept `power_cut_start_box_test` as the fast presubmit smoke gate.
  - `daemon/store/tests/power_cut.rs`: merged 3 matrix tests into 1 `[Host, Machine]` pass with 16 representative live fault points + per-point `SIGKILL` guardrail (11.7 s; removed from `tools/tests/slow.txt`).
  - Trimmed redundant `@n` points and sleeps in `remove_machine_cut.rs`, `upgrade_cut.rs` (also fixing Test 4 to use distinct `HUMATAD2_FAULTS` vs `HUMATAD2` binaries so rollback actually runs), `delete_sessions_cut.rs`, `restore_channel_cut.rs`, `desk_start_host_cut.rs`, `transfers/tests/power_cut.rs` (12.2 s), and `holder/tests/power_cut.rs` (1.4 s).
- **Reverse:** revert the P1–P9 test trims and restore `power_cut_focus.rs` and `store:power_cut_test` in `tools/tests/slow.txt`.

## D889: Desktop notifications notify only for what needs you, behind one master switch, with 30 s frontmost-aware burst collapsing (judged by Gemini, 2026-11-19). Provisional

- **Events that notify (on live state transitions after initial load and `Event::Resync`; never for `answer.is_some()`):**
  - **Inbox requests + agent questions (Yes):** newly arrived pending Agent-inbox `Request`s (`Command`, `FileEdit`, `Network`, `Merge`, `Destroy`, and `RequestKind::Question`), reusing `humata2_inbox::notification` (frame 209) for title/body and opening `Route::Session(session)` with `Panel::Inbox` + `inbox.selected = Some(request)` on click (falling back to `Route::Inbox` + `InboxAction::Select(request)` if the session isn't in the workspace); plus transitions into `Status::NeedsYou` for a session that has no pending Agent-inbox `Request` (covering D845 terminal-only prompts, opening `Route::Session(session)`).
  - **Failed resumes and crashes (Yes):** `session.resume_failed` transitioning from `None` to `Some(_)`, abnormal `Status::Exited { code }` with `code != 0` (when not already covered by `resume_failed`), and `Event::Restart` arriving with non-empty `not_brought_back` (single-session failures open `Route::Session(session)`; multi-session restarts open `Route::Home`).
  - **Excluded (No):** finished agent turns (`Turn::Ended`), PR `checks` strings, `ChannelSetup` step failures, `board.*` to-do asks (D846), and `RequestKind::Migrate` items.
- **Single switch & relation to `permission_request_alerts`:**
  - Settings › Notifications exposes **one** master switch for desktop notifications (`Preferences.notify.enabled`, default `false`). `NotifyConfig` in the leaf crate (`enabled: false`, `inbox_requests: true`, `agent_questions: true`, `failed_resumes: true`, `finished_turns: false`, `migrate_items: false`, `quiet_when_frontmost: true`, `cooldown_secs: 30`) keeps per-category flags explicit for pure unit tests and reversible expansion.
  - Existing `Preferences.permission_request_alerts` (default `false`) stays independent and controls **in-app toasts** only (`features/app/src/events.rs`), orthogonal to macOS desktop notifications.
- **Quiet and burst-collapsing rules:**
  - **Frontmost-on-item suppression:** suppress posting when `app_active == true` and the window is already viewing that target (`Route::Inbox`, `Route::Session(s)` with `Panel::Inbox`, or `Route::Channel(c)` on its `Inbox` tab for an Agent-inbox request; `Route::Session(s)` for terminal `NeedsYou`/`resume_failed`/non-zero `Exited`; `Route::Home` for a multi-session `Restart` summary). Backgrounded/inactive windows always notify regardless of the open route.
  - **30 s cooldown & collapsing:** deduplicate active episodes by `RequestId` / session state; first event for a session posts immediately under `humata.session.<session_id>`; subsequent events for that same session within `30 s` replace in place (`"<N> items need you in <session>"`); 2+ distinct sessions in the same channel `C` within `30 s` collapse into `humata.channel.<channel_id>` (`"<N> sessions need you in #<channel>"` / `"<N> sessions need you on Desk"`); `Event::Restart` with `not_brought_back.len() > 1` posts one summary under `humata.restart.<machine_id>`. Navigating to/answering resets cooldown, and resolving all pending items withdraws its identifier.
- **Settings › Notifications copy (`Row::DesktopNotifications` + `Row::PermissionRequestAlerts`):**
  - Intro caption: `"Requests that need you always appear in the Agent inbox and beside the session in the sidebar."`
  - Row 1 (`settings-desktop-notifications`): title `"Desktop notifications"`, subtitle `"Post macOS notifications when an agent needs your answer or fails to resume. Quiet while you are looking at that item."`
  - Row 2 (`settings-permission-alerts`): title `"Permission request alerts"`, subtitle `"Show brief in-app toasts for new Agent inbox permission requests. Choose where approvals appear in Agents."`
- **Reverse:** add per-event UI toggles in `ui/settings/src/settings.rs` backed by the existing `NotifyConfig` flags, flip `finished_turns` or `migrate_items` to `true`, or merge `permission_request_alerts` into `NotifyConfig`.

## D891: Agents on remote machines get the same browser access as a local session (owner, 2026-11-20)
- **Ruling:** in BROWSER (0.7.0 plan), a session on a remote machine reaches the host Mac's browser with the same switches, grants and Inbox requests as a local session. No extra remote-only gate.
- **Why:** the owner wants machines to be interchangeable; Fable's review had raised remote tab reads as an owner question.
- **Still true:** request cards name the machine the agent runs on, like every other request.
- **Reverse:** add a per-machine browser switch in Settings › Agents, off for remote machines.

## D892: Auto-migrate is this Mac's daemon, CLI and host tools only (reviewer, 2026-11-20). Provisional, open owner question
- **Ruling:** a Migrate item migrates with no click only for the daemon, the CLI and the host tools on this Mac (the local watcher, `svc_upgrade::local`). Holders, on this Mac or a remote machine, and every item about a remote machine (daemon, CLI, host tools, holders) wait for the owner to click Migrate in the Inbox.
- **Still true:** Migrate items are posted, updated and withdrawn exactly as before; `HUMATA2_AUTO_MIGRATE=0` and the `failed.json` breaker still govern the local auto-migrate; once the owner clicks, holders move or restart as before, and the reported failure still trips the breaker.
- **Code:** `Migrations::auto_migrate` accepts only Daemon, Cli and HostTools; `Watch` (remote link-up, refusals, holders reports) no longer starts a migration, and the newer-remote skip (`is_host_older`) went with it; the local holders watch carries on only after the click.
- **Owner question:** should holders, or remote machines, ever migrate without a click? Until the owner rules, they wait.
- **Reverse:** add `Holders` back to `auto_migrate`'s list and call `auto_migrate` again from `Watch::{unsupported, holders, compare}` (with the newer-remote skip) and from the local holders watch.

## D893: The no-login local read tier is status and lists only (reviewer, 2026-11-20). Provisional, open owner question
- **Why:** 0.5.2 added `LOCAL_READ_ONLY` (`daemon/humatad/src/identity.rs`): about 60 read RPCs a same-user process outside every session could call without `humata login`, conversations, turns, search, diffs, comments, Inbox requests and audit among them. D41 (owner ruling) says no credential, no access.
- **Ruling:** trimmed to what `humata status` and the list commands show: `GetServerInfo`, `ListChannels`, `ListSessions`, `ListTabs`, `ListMachines`, `ListAgents`, `ListOperations`. Everything that reads contents needs `humata login`. `tests/auth.rs` checks a conversation and the Inbox are refused.
- **Owner question:** D41 reads as "nothing without a login". Should even these seven stay open?
- **Reverse:** put the removed routes back in `LOCAL_READ_ONLY`, or empty it to follow D41 exactly.

## D894: Frame 12's golden shows the TODO2 drawer; frame 44's repo rows go back to the golden (look_test, 2026-11-20). Provisional, Fable judged
- **Why:** look_test failed on two frames. On `12-panel-full-screen-board`, 5.85% of the sidebar strip differed because the golden predates TODO2: the T1 Need field and loose due dates, the T2 `Won't do` state, and the T4 unset-field highlight. On `44-new-channel`, 3.54% differed because REPOAUTO's keyboard highlight (`.px(6).py(2)` on each repo row) made the rows 4 pt taller and moved them 6 pt right. No decision signed off that layout change.
- **Ruling:**
  - Frame 12: the design drawing (`desktop/tests/design/todo_more.rs`, `panel_drawer_full`) now draws the shipped narrow drawer, and the golden is re-recorded. The rows are State with `Won't do`, Need inside a dashed unset box, and Due as a date box with Today/Tomorrow links on one line and `Loose:` chips on a second. Segments wrap with 7 pt padding. The hint stays "tomorrow" because the fixture day is Thu 19 Nov. Fable approved it as faithful to the app with no clipping or regression.
  - Frame 44: the app is fixed (`ui/core/src/repo_search_field.rs`). The row highlight reaches past the text with negative margins, so rows keep the golden's places, and the list and its clip widen by the same amount. No mask and no budget change.
- **Reverse:** restore the old `12-panel-full-screen-board.png` and `panel_drawer_full`, and put the row padding back in `repo_row`. Either way, look_test fails again until the other side is changed to match.

## D895: FUSEFS mount is off by default; composed agent homes only for new launches (minion FUSEFSWIRE, 2026-11-20). Provisional
- **Why:** FUSEFSWIRE wires `humata2_fusefs` into humatad2 as a fifth helper. Mounting runs the system mount tool. On a Mac without the file-system extension, or in a test, that is a side effect nobody asked for.
- **Ruling:**
  - `Settings.fusefs_enabled` defaults to false. While it is off, humatad2 spawns no helper and mounts nothing. Turning it on in Settings › Sync starts the helper. Turning it off unmounts.
  - Local session folders stay on native disk either way.
  - Sync (`config/config-sync.toml`) follows the composer's defaults: on, with skills and instructions on and MCP and settings off.
  - An agent tab gets a composed `config/<agent>/` home only when it is launched or relaunched after this ships. The home it got is recorded with the tab, so resume and history read the same place.
  - Running holders are never restarted. The sync switch re-composes each home in place.
- **Reverse:** default `fusefs_enabled` to true in `settings_service/sections.rs`. To make sessions that predate this use composed homes too, compose for them at resume.

## D896: BROWSER's bridge and Settings RPCs live in a new `browser.proto` (minion BROWSER, 2026-11-20; main approved)
- **Ruling:** `api/proto/humata/v1/browser.proto`, built in `inbox_proto`, holds `BrowserBridgeService`: `ConnectBrowserHost`, `ExecuteBrowserAction`, `GetBrowserBridgeStatus`, and the user-only `ListDetectedBrowsers`, `InstallBrowserHost` and `UninstallBrowserHost`. `SettingsService`, `settings.proto` and `settings_service.rs` are not changed.
- **Why:** another lane holds `settings_service.rs`. Adding RPCs to `SettingsService` would force edits there.
- **Reverse:** move the three Settings RPCs into `SettingsService` once `settings_service.rs` is free.

## D897: A browser card shows what runs; nothing reaches the browser without the Inbox (minion BROWSER, 2026-11-20; main approved)
- **Ruling:**
  - `RequestBrowserApproval` carries the action's JSON. The daemon works out the card's preview, read or write, and a sha256 action key from that JSON, never from text the agent supplies.
  - An agent's `ExecuteBrowserAction` runs only in one of two cases:
    - It names a request its own session made, which the user approved, for an action with the same key. Each approval is used once.
    - The action is a read, and the session has an active 30-minute read grant.
  - Read grants belong to one session, are kept in memory only, and end after 30 minutes or when humatad2 restarts.
  - Write cards take only Allow once or Deny. Allow 30 min is refused on writes, and Always allow on every browser card. Approve all skips browser cards.
- **Why:** an agent must not be able to show one action on the card and run another, or call the bridge directly. A grant per agent kind would let every session of that agent read tabs.
- **Reverse:** keep grants per agent kind, as `browser_copilot`'s `PermissionGate` did, and let the agent send the preview text.

## D898: The browser host is not a new `Caller` (minion BROWSER, 2026-11-20; main approved)
- **Ruling:** `identity.rs` accepts the `humata-browser-host-token` header only under these conditions:
  - the token matches `<state_dir>/browser-host.token` (mode 0600);
  - the peer has the same uid and is outside every session;
  - the method is `ConnectBrowserHost`.
  It then marks the call with a type local to humatad. `daemon/ports`' `Caller` gets no new variant.
- **Why:** many matches across the daemon list every `Caller` variant. The host may make only one call, so a marker is enough.
- **Reverse:** add `Caller::BrowserHost` to `daemon/ports` and give it a branch in every match on `Caller`.

## D899: `browser_guide_highlight` is a read; `browser_copilot` loses its panics (minion BROWSER, 2026-11-20; main approved)
- **Ruling:**
  - `ActionClass::classify` treats `HighlightGuide` as a read, as the BROWSER brief's table says. It only draws an overlay and changes nothing on the page.
  - `PermissionGate::resolve_ask` drops its `expect` and `unreachable!`.
  - `Submit` stays in the crate but is offered as no tool.
- **Reverse:** classify `HighlightGuide` as a write, so its cards offer only Allow once and Deny.

## D900: SCREENPARSE records Claude Code, Codex and Copilot; agy waits for an API key (minion SCREENPARSE, 2026-11-20; Fable). Provisional
- **Models:** Claude Code on `claude-haiku-5-5`, Codex on `gpt-6-luna` (its fast, affordable current model), Copilot on its cheapest listed model, agy on `gemini-3.8-flash-low` (Fable: the lowest tier of the cheapest family that still calls tools; step up to `-medium` only if it calls none).
- **agy is not recorded yet.** It keeps its files under `$HOME/.gemini` and has no config-dir flag or variable, so a scratch `HOME` isolates its files. But its sign-in is a login-keychain item, not a file: a scratch home starts signed out, and pointing it at the real keychain would let it rewrite the owner's login on token refresh. agy screens are recorded only once a `GEMINI_API_KEY` is passed in the environment (never a file).
- **Widths:** each labelled screen is captured at 60, 80, 120 and 200 columns by resizing the running holder, not by re-running the turn at each width, so a script costs one set of model calls.
- **Not recorded:** a rate limit (it can't be triggered on purpose without burning quota). "Answered from a different terminal" looks the same on screen as an answer typed in the tab, so the after-answer screens cover it.
- **Isolation:** every CLI, including its `--version` call, runs with `HOME` and its config-dir variable pointed at a scratch folder, because Codex writes helper folders under `$HOME/.codex/tmp` even when `CODEX_HOME` is set. The before/after check has no exclusions for `~/.codex/tmp`. Two early runs, before this fix, left `~/.codex/tmp/arg0-macos/codex-arg0{0PsbI3,wT0FaD}` behind. They hold only Codex's temporary helper links, and we leave them untouched.
- **Reverse:** record agy against a copied keychain login, or re-run each script per width.

## D901: A step's context is edited in place in the item drawer (minion TODO2WIRE, 2026-11-20; Fable). Provisional
- **Control:** a step with no context gets a "+ Context" link before its ↑ ↓ Drop. It shows only while the pointer is over the step or the link has focus (`todo-step-context-add-{i}`); it is always laid out, at opacity 0 at rest.
- **Existing context:** clicking a step's grey context line (`todo-step-context-{i}`) edits it.
- **Editor:** both ways open one multi-line box under the step (`todo-step-context-edit`, 3 to 8 rows). It has Save (`todo-step-context-save`), Cancel (`todo-step-context-cancel`) and the hint "⌘↩ saves · Esc cancels".
- **Keys:** Return adds a new line, ⌘↩ saves, and Esc cancels without closing the drawer.
- **Saving:** empty text removes the context; there is no Remove control. One editor is open at a time, through `EditField::StepContext(i)` and `SetStepMarkdown`.
- **Narrow:** the session panel keeps the context read-only, as it already hides step reordering.
- **Frames:** `workflow/milestones/mockups/todo2wire/t2-0`, `t2-1` and `t2-2`.
- **Why:** nothing is added to a step row at rest, so no design golden changes. A context is usually several lines, so the single-line title field doesn't fit.
- **Reverse:** show "+ Context" permanently (a one-line change) if hover proves hard to find.

## D902: To-do automation is a "To-do" group in Settings › General (minion TODO2WIRE, 2026-11-20; Fable). Provisional
- **Place:** under Fast mode and above Repositories.
- **Header:** a "To-do" heading, with one line saying Ask puts a one-click question on the item and never stops an agent.
- **Card:** one card of five rows:
  - "When a pull request merges", Ask / Auto (`settings-todo-pr-merged-*`)
  - "When an item closes", Ask / Auto (`settings-todo-close-archive-*`)
  - "Wake the agent when checks fail", a switch (`settings-todo-wake-ci`)
  - "Wake the agent on review comments", a switch (`settings-todo-wake-comments`)
  - "Nudge about idle items", a pop-up of 1, 3, 7 or 14 days (`settings-todo-idle-nudge`)
- **Defaults:** Ask, Ask, off, off, 3 days. Auto comes with Undo.
- **Archiving:** Auto archive always keeps a session that has unsaved work or a running terminal, and the daemon enforces this.
- **No Off for the nudge:** the setting has no off value.
- **Frame:** `workflow/milestones/mockups/todo2wire/t7-1-settings-todo.png`.
- **Why General:** a ninth section entry would change every Settings golden for five rows. General is in no golden.
- **Not yet:** channel overrides get no UI.
- **Reverse:** move the group to its own section at the next Settings redesign.

## D903: The PR watcher wakes sessions only for failing checks and review comments (minion TODO2WIRE, 2026-11-20). Provisional
- **What wakes a session:** failing checks follow `auto_wake_on_ci_fail`, and new unresolved review comments follow `auto_wake_on_review_comments`.
- **Always a prompt:** merge conflicts, a merged base and a PR closed without merging show a prompt on the item ("Send to session"), because no setting covers them.
- **Signals:** a prompt is raised only when its signal is new since the last check. A dismissed prompt stays gone until the signal clears and comes back, including across restarts.
- **Cadence:** checks run at most every 5 minutes, only while some item watches a PR, and right away when a PR is added. After a gh rate limit, the watcher waits the stated time (15 minutes when gh doesn't say).
- **Reverse:** let conflicts follow the checks switch.

## D904: Settings › Sync is frame 211; dim, never hide (minion FUSEFSWIRE, 2026-11-20)

Fable signed off candidate `211-settings-sync` with edits.

Layout:
- Sync is third in the Settings nav, after Agents, with the FolderTree icon.
- Card 1 holds the master switch and four rows indented 28: Skills (with a Host wins / Host only control), Instructions, MCP servers, Settings.
- Then a "Machines" table: This Mac first, then the other machines in sidebar order. Each cell is a Default/On/Off popup. Default is drawn in the secondary colour and writes the field as unset. Offline machines are listed and can be edited.
- Then a separate mount card ("Browse other machines as files", off).
- Copy defines the host once: "The host is this Mac." Every session on any machine gets the host's config, which matches config_compose's host-synced vs machine-local layers. The manager corrected Fable's wording ("the machine it runs on") to say this.

Dimming:
- Master off: the category rows and the whole Machines table dim to 0.4, are disabled and keep their values. The master row and the mount card never dim.
- Skills off: only the mode control dims.
- A machine's Sync cell Off: the rest of that row dims.
- Per-machine `skills_mode` is not shown in v1 and stays unset.

Follow-up: goldens 29-settings and 117 draw the nav without Sync. Add it in design/pages.rs and design/agent_asks.rs, and re-record both when the native nav gains Sync.

Sign-off (Fable, 2026-11-20): the render of `211-settings-sync` is approved with the reworded master note, and so are the re-drawn `29-settings` and `117-settings-agents-ask-in-inbox`, whose only change is the Sync nav row and the rows below it moving down one row (29 pt). Record all three.

## D905: The host daemon serves the To-do UI at a local URL, switched in the desktop app (owner, 2026-11-20)

The owner asked for the root/host daemon to host the To-do UI at a local URL so Humata's overall state can be checked in a browser. Built with GPUI's web support; gaps are fixed in our crates. A desktop Settings switch enables and disables it. This narrows the earlier cut of the browser client: only the To-do page, nothing else. It is the very last milestone (WEBTODO, release 0.11.0), after 0.10.0's FUSE work and the other lanes are validated.

## D906: Read-only provider helper commands let an agent write a machine provider from the CLI alone (owner, 2026-11-20)

The new commands are `humata provider guide`, `schema`, `example`, `check`, `render` and `parse`, plus their MCP twins.
- None of them runs, imports or changes anything, so none needs approval.
- `schema` is generated from the parser's own types, so it can't drift from it.
- Importing stays owner-only (MP-23).

It ships as 0.10.1, after FUSEFSWIRE (milestone PROVIDERCLI). The Create channel fix (NEWCHCLOSE) moves to 0.10.2.

## D907: The file-byte MCP tools stay as the fallback when FUSE can't be set up (owner, 2026-11-20)

The retirement of `files_list`, `file_read`, `files_find`, `file_write`, `folder_create`, `file_fetch` and `session_read` (D886's first bullet, and the B2 change that built it) is withdrawn. The mount is off by default and some machines can't run it, so agents need the tools whether or not anything is mounted.
- The seven tools are registered and work with or without a mount; the mount is an additional way to read the same places.
- The humata-files skill says which to use: plain files under `$HUMATA_FUSE_ROOT` when it is set, `humata files` and the MCP files tools when it is not.
- The rest of D886 stands.

## D908: The mount card says where the mount is, or why there isn't one (Fable, 2026-11-20)

The "Browse other machines as files" card in Settings › Sync shows the daemon's live mount state, not only the saved switch. Fable decided this and signs off the design; candidates `211b-settings-sync-mount-cannot` and `211c-settings-sync-mounted` still need their renders judged before they move into FRAMES.

The switch (`settings-fusefs-enabled`) always shows the saved setting. The state adds at most one thing to the card:
- **Off:** nothing. The card is frame 211, unchanged, and goldens 211 and native settings-sync don't move.
- **Starting:** a third secondary note under the two existing ones, id `settings-fusefs-status`: "Mounting…".
- **Mounted:** the same note, same id: "Mounted at <path>", with the home folder written as `~` (for example "Mounted at ~/Humata"). The path comes from the daemon. It is shown because the owner needs it to open the folder, and it is the only sign that the switch worked.
- **CannotSetUp:** no status note. Under the row, inside the card, a needs-you line, id `settings-fusefs-problem`: "<reason> Sessions reach other machines through the files tools instead."

The needs-you line uses the existing look: `attention_bg` behind it, a 12 pt orange Info icon, the sentence in the primary text colour at the page's 12.5 pt, padding 14 by 8, and the card's bottom corners rounded. The sentence is not tinted, because orange text at this size is too faint on a light page.

Rules:
- The reason is the daemon's sentence, shown as sent. It ends with a full stop, says what failed, and says how to fix it when a fix exists (for example "The mount helper wouldn't start. Turn this off and on to try again."). When nothing can be done on this machine it says only what is missing ("This Mac has neither FSKit nor /dev/fuse.").
- Turning the switch off and on again is the retry, so the card has no retry button. Off clears the line at once.
- The line appears only while the switch is on. With the switch off, a machine that can't mount shows nothing, since nothing is blocked.
- The card never dims and nothing else on the page changes in any state. The second note ("Session folders on this machine stay on its own disk either way.") stays in every state.
- A helper that died and came back shows Starting, then Mounted. Only one that could not come back shows the needs-you line.
- Nothing goes to the Inbox, the sidebar or a notification for this: agents already fall back to the files tools (D907), so no work is blocked outside this card.

Sign-off (Fable, 2026-11-20): the renders of `211b-settings-sync-mount-cannot` and `211c-settings-sync-mounted` are approved and moved into FRAMES; record both. Each differs from 211 only inside the mount card. In 211b the card ends half a point above the bottom of the 760-high frame with its border and corners whole, so nothing is clipped.

## D909: HIGHLIGHT ships in 0.10.2 and STACKGH in 0.10.4, before WEBTODO (main, 2026-11-20; reversible)
- The owner asked for both on the roadmap without giving releases.
- **HIGHLIGHT** is a bug fix, so it rides with the other 0.10.2 bug fixes.
- **STACKGH** is new CLI work. It comes after the SMFIX state machine fixes (0.10.3) and before WEBTODO, which the owner wants very last (D905).
- The parity table's differences go to Fable.

## D910: A Desk agent may start or fork sessions in any channel, with the owner's approval (owner, 2026-11-20)
The owner asked: "can we let the global desk make sessions in addition to channels if I approve them?" Before this, a Desk agent's start in any channel was refused, because no channel is its own (D182.3). Now a global Desk agent's start or fork, in any channel and on any machine, goes through the matching "Start or fork sessions" rule under the Desk's new-channel defaults: Ask me, so it lands in the Inbox. The ask is filed under the desk channel id and carries the target channel and machine. Never still wins. Neither DK-4 switch allows it, because the owner said "if I approve them". A fork also reads the parent's conversation, so it is refused when the Desk's other-channels read rule is Never. Channel Desk sessions keep their own rules (CD-3). Ordinary, fork and review sessions are still refused outside their channel. MCP `session_start` takes an optional `channel`, which a Desk agent must send, and `session_fork` takes an optional `session` to fork. The start and fork cards show the target channel and machine. **Always allow** stays per machine and agent, so for a Desk agent it covers every channel, and the option's text says "in any channel" (main's pick; reversible: add a channel to the grant target). **Reverse:** put the `in_channel` check back for `SessionKind::Desk` in the policy's start arm.

## D911: A real FSKit mount on the Mac is its own milestone, FSKITSERVE, in 0.10.5 (main, 2026-11-20; reversible)
FUSEFSWIRE found that `humata2_fusefs serve` served no volume on either OS:
- On Linux it ran `/sbin/mount -t linux …`, which always fails. FUSEFSWIRE fixes Linux itself, with a fusermount3 mount, the FUSE wire codec and a serve loop in `libs/fusefs/linux`, and a `LinuxFuseBackend` in the adapter.
- On the Mac a real mount also needs the FSKit appex, its signing and entitlements, and the bridge server started in `run_serve`. That is too much for 0.10.0.
Until FSKITSERVE ships, a Mac reports "can't set up" with a reason and agents use the files tools. That is the owner's fallback rule.
**Reverse:** pull FSKITSERVE into an earlier release.

## D912: A To-do ask lives on the To-do only, and notifies as a To-do (Fable, NOTIFY2, 2026-11-20)
The owner said: "stuff that goes to TODO should not go to inbox, like status change requests." A To-do ask is an agent's suggested new item or a change to an item: a status, a close or a step. The daemon has left these out of Agent-inbox lists since D846, but the live event stream still put them in the app's requests. The app recognised them only by a `board.` operation, which To-do service asks don't have, so they showed as Inbox cards, counts and Inbox notifications.
- The client now marks them from the preview kind, the same test the daemon applies (BoardChange or NewBoardItem): `Request.to_do: Option<ToDoAsk>` and `Request::is_to_do_ask()`.
- They are left out of `Workspace::waiting()`, the Inbox's cards and chips, Home's Needs you, and the badges. The To-do still shows and answers them on the item.
- A To-do ask still posts a notification, under the master switch with no extra toggle:
  - title "<agent> suggests a To-do: <item>", or "<agent> wants to change '<item>'";
  - body "Accept or dismiss it on the To-do.";
  - clicking it opens the To-do on the item (`NotifyTarget::Todo`);
  - it stays quiet while the window is frontmost on the To-do.
- A To-do ask stays To-do-only even when its agent is blocked, because one ask gets one place to answer.
- **Reverse:** make `is_to_do_ask()` return false.

## D913: Notifications ask for permission when switched on (main's diagnosis, NOTIFY2, 2026-11-20)
The installed app never posted a notification. The switch is off by default, and turning it on called a permission request that the desktop notifier ignored. So macOS never asked, the app never appeared in System Settings › Notifications, and the first post could be lost while permission was still pending.
- The desktop notifier now asks for permission when the switch is turned on, through a small patch to the UI framework: `request_system_notification_permission`.
- The same patch adds `show_system_notification_with_sound` and lets a notification play its sound while the app is frontmost.
- The patches are in `engine/patches/gpui-{notification,macos-notification}-sound.patch`, and the `UNNotificationSound` feature is on.
- **Reverse:** drop the two patch annotations in MODULE.bazel and go back to `show_system_notification`.

## D914: Two notification sounds of our own, behind Play a sound, off by default (Fable, NOTIFY2, 2026-11-20)
The owner asked for "a noise ... One for INBOX and another for TODOs ... pleasant and gentle please and short please."
- `app/sounds/make_sounds.py` makes both sounds from sine partials and is deterministic:
  - `humata2-inbox.wav`: E5 rising to A5, 0.38 s, peak -12 dBFS.
  - `humata2-todo.wav`: one G4, 0.30 s, peak -16 dBFS, lower and softer.
- `app.bzl` copies them into Contents/Resources.
- With Play a sound on, a To-do ask plays the To-do sound and every other notification plays the Inbox sound. A notification within the cooldown of the last sound posts silent, so a burst never stacks sounds.
- Fable judged both from their parameters and waveform: no clicks, soft attack, distinct. It raised both levels 4 dB.
- The "Play a sound" row comes in Settings › Notifications after "Desktop notifications". It is greyed while the master switch is off.
- **Reverse:** set `play_sound` to false, or delete the `sounds` attr default.

## D915: A default never lands on an agent that is off; with none on, the app says so (Fable, AGENTOFF, 2026-11-20; reversible)
The owner reported: "when i disable claude and codex it defaults to them and fails because they dont exist."
1. **One rule.** A start that names no agent takes the first agent that is on along its chain. The daemon's chain is the channel's own default, then Settings', then the one the channel was made with. The app's chain is the channel's start agent, then Settings'. After the chain, it takes the first agent that is on in Settings › Agents order. The plain shell counts only when it is named. The rule is `AgentSwitches::pick` in the daemon (`agent_gate::pick`) and `humata2_clients::pick_agent` in the app.
2. **Stored defaults are kept.** A Settings default, a channel default or the Desk's `--default-agent` that is off stays saved and falls back at use time, so turning the agent back on restores it. A start that explicitly names an agent that is off is still refused.
3. **Fork.**
   - The Fork dialog of a session whose agent is off preselects the first agent that is on, opens More options, and says "Claude is off, so this fork starts with Copilot."
   - The daemon refuses a fork that names no agent while the parent's agent is off ("name an agent that is on for the fork, or turn X on in Settings › Agents"). It never switches agents silently, because a different agent changes what the fork carries.
4. **With every agent off:**
   - New session opens on Terminal with a note, not an error: "Every agent is off. Terminal still starts a plain shell; to start an agent, turn one on in Settings › Agents."
   - New desk session, Fork and Review a pull request open nothing. A toast says "Every agent is off. Turn one on in Settings › Agents, then try again." and has an Open Settings button.
   - The daemon refuses a start that names no agent: "Every agent is turned off in Settings › Agents, so Humata has none to start". The fix names Settings › Agents, or `zsh` for a plain shell.
5. **Continue in another channel** for a session whose agent is off is refused, because the conversation belongs to that agent: "<Agent> is turned off in Settings › Agents, so this session can't continue in another channel".
6. **Settings › Agents › Default agent:** when the saved default is off, a dim line under the buttons says: "Claude, your default, is off. New sessions start with Copilot until you turn Claude back on or pick another." When every agent is off, it says: "…, and so is every other agent. New sessions can only start Terminal until you turn an agent on."

**Reverse:** make `pick` return the first preferred kind whatever its switch says.

## D916: A session's shell gets `HUMATA_FUSE_ROOT` and `HUMATA_MACHINE_NAME` too, but no config home (FUSEFSWIRE, 2026-11-20; reversible)

- A Terminal session, and a shell tab inside any session, is started with the same place variables as the session's agent: `HUMATA_FUSE_ROOT` while the tree is mounted, and `HUMATA_MACHINE_NAME`. It gets no composed config home, since no agent runs there.
- A machine shell belongs to no session and gets neither. A session on another machine gets nothing from this daemon, as before.
- Why: the mount is how anything in a session reaches the other locations, and the owner works in shells as much as agents do. Before this, only agent tabs got the variable, so a shell had no way to find the mount.

## D917: SCREENPARSE `record_corpus.sh` is unsafe to rerun across all agents until `HOME` and cache isolation are fixed (Judge, 2026-11-20)
- **Ruling:** `UNSAFE` until 4 exact lines in `daemon/svc_inbox/tests/screenparse/record_corpus.sh` are fixed.
- **Codex-only note:** under `env -i HOME=$HOME USER=$USER PATH=$PATH TMPDIR=$TMPDIR`, `codex` (`version_of` lines 65/69 and `setup` line 113 setting `HOME="$home"` and `CODEX_HOME="$home/codex"`, with line 109 only reading `$HOME/.codex/auth.json`) no longer writes to `~/.codex`.
- **Lines to fix before rerunning `record_corpus.sh`:**
  - `record_corpus.sh:99` (`claude`): `ENVS` sets `CLAUDE_CONFIG_DIR` without `HOME=$home`, leaving `claude` and subprocesses running against real `$HOME`.
  - `record_corpus.sh:70,132-133` (`copilot`): omits `HOME=$home`, `COPILOT_CACHE_HOME` / `COPILOT_PKG_CACHE_HOME`, and `--no-auto-update`.
  - `record_corpus.sh:65-66,79,146`: unset/clear `GEMINI_CLI_HOME`, `COPILOT_CACHE_HOME`, `COPILOT_PKG_CACHE_HOME`, `XDG_*`, and `CLAUDE_SECURESTORAGE_CONFIG_DIR`.
  - `record_corpus.sh:54-57` (`real_changes`): include `"$HOME/.gemini"` in `dirs`, catch new directories as well as `-type f`, and drop broad exclusions that mask real-home writes.

## D918: `providers-e-remove-blocked.view.txt` `Tab "Sync"` diff is approved (Judge, 2026-11-20)
- **Ruling:** `INTENDED (APPROVED)`.
- **Why:** `Tab "Sync"` at line 51 of `desktop/tests/__snapshots__/native/providers-e-remove-blocked.view.txt` (between `Tab "Agents"` and `Tab "Machines" selected`) directly reflects adding `(Section::Sync, IconName::FolderTree, "Sync")` 3rd in the Settings navigation rail (`ui/settings/src/settings.rs:662`) under D904. Both the `.view.txt` and `.png` render cleanly with zero regressions.

## D919: `live_diff::spec_to_wire` uses `Upto::Commits` + `Untracked::None` for pinned `Range`/`Overlay` specs (2026-11-20; reversible)
- **Why:** Opening a prepared stacked `ReviewPlan` view sent `RepoBase::Range` / `RepoBase::Overlay` with hardcoded `Upto::Unstaged` + `Untracked::All`, causing `daemon/review_diff` (`Checkout::pinned_tree`) to refuse every diff with `ConflictingCompare` ("mixes a head or worktree paths with other levels") and leaving the Diff panel stuck.
- **Fix:** `clients/live_diff/src/diff.rs` sets `upto = Upto::Commits` and `untracked = Untracked::None` when `each.base` is `Range` or `Overlay`, keeping `Unstaged` + `All` for unpinned bases.

## D920: Pushing a session's non-default branch does not ask for permission (owner, 2026-11-20; queued for 0.10.1)
- **Owner's words:** "humata should not ask for permission to push up branches just saying."
- **Plan (0.10.1):** Allow non-force branch-only pushes on non-default session branches (`open_pull_request = false`) without an Inbox approval card, while keeping `SessionKind::Review { push_allowed: false }` refused, keeping protected/default branches (`main`/`master`/`origin/HEAD`) and force-pushes guarded, wiring MCP `session_push` in `svc_mcp`, and auto-allowing safe `humata session push` / non-force non-default `git push` in agent hooks.

## D921: 85 updated native UI snapshots for Settings/Channel UI, Diff View UX, and `HIGHLIGHT` are approved (Judge, 2026-11-20)
- **Ruling:** `APPROVE` across all 85 modified `.png` and `.view.txt` snapshots in `apps/humata2/desktop/tests/__snapshots__/native/` (excluding pre-existing foreign `away-left-out-before.png`).
- **Why:** Visually and structurally verified zero regressions across all three intentional changes: (1) `Settings › Browser` tab, `Channel › Settings` tab, and `AGENTOFF` dim fallback note (`"Off: it asks in its own terminal"`); (2) short repo names when unique with `org/repo` collision disambiguation (`acme/ledger` vs `ledger` in `stack-layer-counts-full`) plus `Splitter "Resize the file list"` in full-screen diff views; and (3) multi-color syntax highlighting in light and dark themes across diffs (preserving word-diff overlays and wrapped lines), inline code editor, and Markdown source panes.

## D922: 25 updated panel and native UI snapshots for Palette/Sidebar UX, Shortcuts, and Back-button polish are approved (Judge, 2026-11-20)
- **Ruling:** `APPROVE` across all 25 modified `.png`, `.txt`, and `.view.txt` snapshots in `apps/humata2/desktop/tests/__snapshots__/{panels,native}/`.
- **Why:** Verified zero visual regressions, clipping, or layout breaks across: (1) short repo name `api` + syntax highlighting + `Splitter "Resize the file list"` in `panels/changes-{light,dark,full-light,full-dark}`; (2) `30-palette` scrollable `ListBox`, `#palette-footer` keyboard legend (`> commands   # channels   @ sessions   / files`), and unchecked `Include archived` toggle; (3) `shortcuts-sheet-{light,dark,scrolled-light}` showing `⌘B ⌘0` (sidebar) and `⌘+ ⌘-` (font zoom); and (4) hiding dead disabled `<` Back buttons when `state.trail.is_empty()` (`31-new-session`) while keeping active Back buttons when navigated from Home (`session_lifecycle`, `session_menu`).

## D923: 5 updated Channel UI snapshots with `Tab "Desk"` first and default are approved (Judge, 2026-11-20)
- **Ruling:** `APPROVE` across `18-channel-sessions`, `19-channel-activity`, `20-channel-repos-machines`, `35-empty-channel`, and `channel-agent-access-rows` (`.png` + `.view.txt`).
- **Why:** Verified `Tab "Desk"` sits cleanly in the first slot with zero wrapping or clipping, `ensure_channel_desk` auto-starts the channel's primary Desk on arrival, titlebar and body pixels are unchanged, and all `look.rs` golden budgets stay under 0.500%.

# ASK: Agents ask, I decide

- **Status:** in progress (lane ASK)
- **Depends on:**
  - FL-D: an agent's MCP identity is its real session
  - FL (to close, not to open): flow §26.4 in the app needs a real session on screen
  - LOOK1 (to close, not to open): the session Inbox panel is where the card is drawn
  - FRAMES1 (to close, not to open): the channel's Inbox and Agent access tabs have no frame yet
  - W1 (to close, not to open): the card in frames 03, 04, 26, 27 is redrawn; match it before this closes
- **Unblocks:** BOARD, FORK, AWAY, VIEWS, PR, LINUX
- **Layer:** L1 · **Lane:** daemon, then UI

## Outcome
An agent calls a Humata tool. A card with full context appears. Allow, Deny or Reply…, or Always allow exactly this. The agent continues. Rules are set per channel.

## Exit criterion
Daemon first: the capability matrix runs as a boundary suite over MCP and the CLI; Ask becomes an Inbox request that waits and can be collected; grants are scoped, revocable and survive a restart; an added agent's calls are refused (SE-29). Then the app: flow §26.4 against a real daemon, the card carries every part of IN-9, and the Agent access tab edits rules.

## Read these (pointers, nothing else)
- SPEC: IN-1–IN-26, TP-1–TP-17, SEC-3, SEC-7–SEC-12, SEC-15, SE-20, SE-29, ST-4; flow §26.4. Find each with `grep -n '\*\*<ID>\*\*' apps/humata2/SPEC.md` and read the few lines around it.
- Frames: 03, 04, 26, 27 (as redrawn by W1), and the channel Inbox and Agent access frames from FRAMES1 (`desktop/tests/__snapshots__/design/NN-*.png`)
- Decisions: D105, D103, D32, D33, D37, D38, D41, D48, D90, D91, D93, D96 (grep `^## D<n>:` in DECISIONS.md)
- Code (under `apps/humata2/`): `daemon/access`, `daemon/mcp/src/tools.rs`, `daemon/humatad/src/inbox.rs`, `daemon/humatad/src/access_service.rs`, `daemon/humatad/src/mcp.rs`, `daemon/humatad/tests/inbox.rs`, `daemon/humatad/tests/mcp.rs`, `cli/src/commands/inbox.rs`, `features/inbox`, `ui/src/panels.rs`
- API: `api/MCP.md`, `api/CLI.md`, `api/BOUNDARIES.md`; `api/proto/humata/v1/inbox.proto`, `access.proto`, `capabilities.proto`
- Also: OVERVIEW.md › Known gaps › "MCP, access and other open items"
- P0 is splitting some of these files; if a path is gone, look for its parts beside it (`ls` the folder).

## Out of scope
The While I'm away rule and journals (AWAY). Board requests answered on the item (BOARD). Contain agents (SEC-6) and script tokens (SEC-14): not in 1.0.

## Lane (written by the manager)
- Design notes (daemon part, batch 1):
  - Ask on an ordinary call (D38.6 closed): a handler whose `Guard::check` says `Checked::Ask(card)` hands the card and a boxed "run it" future to `Inbox::defer`; the Inbox keeps the request (same idempotency key → same request, TP-14), returns `status::approval_pending(request)` (FailedPrecondition, `Error.kind = ApprovalPending{request}`, seam S1), and on Allow runs the call in a detached task whether or not anyone waits (TP-13), keeping the result bytes + type for 24 h in `WaitResponse.result`. Approving when the rule is now Never fails (IN-21). After a restart the continuation is gone: the answer is kept, a wait returns `request_withdrawn` with a fix telling the agent to repeat the call with the same key (decision A1).
  - MCP turns `approval_pending` into `Reply::Pending{request}` (the mcp crate already waits); the CLI turns it into exit 12 `pending` with the request id.
  - Rules: GetChannelRules, SetChannelRule, GetDefaultRules, SetDefaultRules, ListCapabilities, Explain built in `access_service.rs`; rules saved to the store as `humata.v1.ChannelRules` / `DefaultRules` bytes (no new store proto, D96 gap closed) and restored at start; only the user may change them (IN-23); changes apply from the next call (IN-21) and go out on the feed.
  - Grants end when their session is archived or deleted (IN-22, `Policy::end_grants_of`); a stopped/deleted session's pending requests are withdrawn (TP-15).
  - SE-29: an added agent's tab gets a token whose caller is `Caller::AddedAgent{session}`; every call but WhoAmI is refused with "a fork gives this agent a session of its own". Opening an added-agent tab needs a terminal.proto RPC (seam S3) and store TabKind (seam S4).
  - Boundary suite: `daemon/humatad/tests/matrix.rs` runs TESTING.md §5.5 tiers over MCP and the CLI (Board tier skipped until BOARD: BoardService isn't built).
  - SEC-15 (identity used outside the process tree) is batch 2: turning it on breaks every test that uses a tab's token from the test process (tests/mcp.rs Fixture), so those move to running inside the tab first.
- Claims (all under `apps/humata2/`, clean in git status when claimed): `daemon/humatad/src/{inbox,inbox/kept,inbox/deferred,guard,status,channel_service,access_service,rules_kept,identity,restart,session_service,mcp}.rs`; `daemon/humatad/tests/{inbox,access,auth,sessions,mcp,matrix}.rs`; `daemon/humatad/tests/common/mod.rs` (additive only); `daemon/store/src/{rules,schema,lib}.rs`; `daemon/store/BUILD.bazel`; `cli/src/{error,call,exit}.rs`; this Lane section
- Tasks:
  - [ ] T1 Ask → Inbox for ordinary calls, ListChannels first (D38.6, IN-6, IN-12, IN-15, IN-21, TP-12–TP-14) · worker defer (Sonnet, medium) · files `daemon/humatad/src/{inbox.rs,inbox/deferred.rs,inbox/kept.rs,guard.rs,status.rs,channel_service.rs}`, `daemon/humatad/tests/inbox.rs` · done (uncompiled): `Inbox::defer` + `deferred.rs`, ListChannels asks under GlobalRead (default Ask), Allow runs the call detached and keeps the result 24 h in memory, IN-21 Never re-check, `withdraw_pending_of` for T3; deferred cards carry no preview and no Always allow yet (batch 2)
  - [ ] T2 Rules RPCs + rules kept across restart (IN-16, IN-17, IN-21, IN-23, ST-4) · worker rules (Sonnet, medium) · files `daemon/humatad/src/{access_service.rs,rules_kept.rs}`, `daemon/store/src/{rules.rs,schema.rs,lib.rs}`, `daemon/store/BUILD.bazel`, `daemon/humatad/tests/access.rs` · done (uncompiled): six RPCs built (all user-only via AccessChange; agents use Explain), rules in new store tables `access_channel_rules`/`access_default_rules` (schema bump), restore fails loudly on unreadable rows; only channels with a user-set rule are kept; SetDefaultRules has no feed payload
  - [ ] T3 Grants end with archive/delete, requests withdrawn on stop/delete, added agent refused (IN-22, TP-15, SE-29) · worker sessions (Sonnet, medium) · files `daemon/humatad/src/{identity.rs,restart.rs,session_service.rs}`, `daemon/humatad/tests/{sessions,auth}.rs` · done (uncompiled): stop withdraws pending (needs `Inbox::withdraw_pending_of`, queued for worker defer); `end_grants_of` helper ready but archive/delete RPCs aren't built so nothing calls it yet; SE-29 refused at the interceptor via `TabIdentity::AddedAgent` (no `Caller` variant: would touch 8 unclaimed files), token file gains an optional 4th word
  - [ ] T4 MCP tools for the matrix + pending over MCP · worker mcp (Sonnet, medium) · files `daemon/humatad/src/mcp.rs`, `daemon/humatad/tests/mcp.rs` · done (uncompiled): channels_list, session_info/stop/archive, files_list/read/write, inbox_withdraw/mine, access_explain wired; ApprovalPending → Reply::Pending; hand-rolled base64 in mcp.rs `mod files` (no base64 dep)
  - [ ] T5 Boundary matrix over MCP and CLI + CLI pending exit 12 · worker matrix (Sonnet, medium) · files `daemon/humatad/tests/matrix.rs`, `daemon/humatad/tests/common/mod.rs` (additive), `cli/src/{error,call}.rs` · done (uncompiled): matrix.rs MCP+CLI tiers (Board ignored), CLI ApprovalPending → exit 12 or waits with --wait; common gains shell_token/humata_as_agent/inbox helpers; manager added the `exit.rs` of_kind arm
- Waiting on: main agent for seams, then a build gate. Seams:
  - S1 `api/proto/humata/v1/errors.proto`: in `Error.kind` add `ApprovalPending approval_pending = 57;`; add `message ApprovalPending { RequestId request = 1; }`
  - S2 `daemon/humatad/src/main.rs`: `mod rules_kept;` (between `mod restart;` and `mod review;`)
  - S3 `serve.rs`: after `let guard = Arc::new(Guard::new(&machine, role));` add `crate::rules_kept::restore(&store, &guard).map_err(/* new DaemonError variant with #[source] RulesKeptError */)?;`; `Access::new(guard.clone(), feed.clone(), inbox.clone(), store.clone())`; `ChannelSvc::new(..., args.default_agent.clone(), inbox.clone())`; `SessionSvc::new(..., machine.clone(), inbox.clone())`
  - S4 `daemon/humatad/src/events.rs` `Filter::passes`, before `_ => false`: `Some(pb::event::Payload::Rules(_)) => self.user && self.access && self.asked(scope),`
  - S5 (later, SE-29 end to end): store `TabKind::AddedAgent` in `daemon/store/src/records.rs` + its encoding; terminal.proto `OpenAgentTab`; then `restart.rs` `tab_identity`: `TabKind::AddedAgent => TabIdentity::AddedAgent`, arms at `terminal.rs:156,191`, mint with `mint_as(.., TabIdentity::AddedAgent)`
- Decisions for the main agent:
  - A1 After a restart an approved Ask can't run (the continuation is in memory): the answer is kept and a wait says to repeat the call with the same key. Reversible: later we can persist calls as replayable requests.
  - A2 Pending over gRPC is `FailedPrecondition` + `Error.kind = ApprovalPending{request}` (new kind 57), not a new response field on every RPC.
  - A3 Rules stored as `humata.v1.ChannelRules`/`DefaultRules` bytes in a new store table, not a new store proto.
  - A4 SEC-15 moved to batch 2 (test fixture impact above).
- Decisions (batch 1, more):
  - A5 Agents' ListChannels now goes through GlobalRead (default Ask), so an agent's channel list waits in the Inbox by default (TESTING §5.5 "Ask me" tier). Before, it was only filtered by ChannelRead.
  - A6 Deferred results are kept in memory only (store InboxEntry has no result field); a pending deferred request is withdrawn (CallEnded) on restore; deferred entries are recognised after restart by having no preview (fragile: a store flag would be better, batch 2).
  - A7 Deferred cards have no preview and no Always allow yet (IN-10, IN-14: batch 2).
  - A8 SE-29 refusal is at the interceptor via `TabIdentity::AddedAgent` in the token file (optional 4th word), not a `Caller` variant; WhoAmI shows an added agent as unauthenticated.
  - A9 Rules reads (Get*) are user-only too; agents read their own access via Explain.
  - A10 Rules blobs stored as raw proto bytes, not the framed blob format; lazily added channels aren't saved; channel deletion doesn't drop the rules row yet.
  - A12 Reads (ListChannels) carry no idempotency key, so a repeated read asks again: TP-14 holds only for calls with a key. The CLI waits on the request its own `--wait` run made; the matrix tests do that now.
  - A11 mcp.rs has a hand-rolled base64 for file_read/file_write (no base64 dep in DAEMON_DEPS); swap for the crate if preferred.
- Owner questions: none.
- Batch 2 (next): SEC-15 process-tree check (+ move tests that use a tab token from outside the tab); IN-22 grants end once archive/delete RPCs exist (`SessionSvc::end_grants_of` waits, `#[allow(dead_code)]`) and publish grant-ended events; previews + Always allow on deferred cards; more handlers deferred (sessions, files); then the app part (§26.4, IN-9 card, Agent access tab).
- Build gates: gate 1 (main): all build; 28/29 pass; matrix_test 3 cases failed (MCP call waited the 45 s default; CLI reran a keyless read expecting the same request) → test fixes in matrix.rs, rerun pending
- Ready to commit: not yet: after S1–S4 and a green gate. Builds: //apps/humata2/daemon/humatad:humatad2 :humatad2_admin, //apps/humata2/cli:humata; tests: //apps/humata2/daemon/humatad:{humatad2_unit_test,inbox_test,access_test,sessions_test,mcp_test,matrix_test,auth_test,channels_test}, //apps/humata2/daemon/store:store_unit_test, cli unit tests; clippy on humatad, store, cli

### Batch 2 (daemon; batch 1 committed 4d873e4c, decisions in D127)
- Design notes:
  - SEC-15: `Authenticate` checks that a session token's caller pid descends from its tab's holder pid (reusing `terminal_session::descends_from`/`Processes`); a mismatch is refused with `identity_mismatch`, the tab's token is marked misused and replaced at the session's next resume, and the misuse is logged (Activity isn't built: logged as `IDENTITY_MISUSED` in the daemon log). Test daemons started with `--admin` (anyone) skip the check so the existing suites that hold a tab token from the test process keep working; `--admin-pid` and release daemons enforce it, and new tests prove both sides (a `humata whoami` typed into the shell tab works; the same token from the test process is refused). (decision B1)
  - Deferred cards get the preview and Always allow offer an ask-ahead card gets (IN-10, IN-14, IN-22 exact target), built inside `Inbox::defer` from the card's call; `defer`'s signature stays.
  - Store flag: `humata.store.v1.InboxEntry` gains `bool deferred = 5;` (seam S6); kept.rs reads it instead of "no preview".
  - Remaining unary guarded handlers in session_service.rs and channel_service.rs defer an Ask the way ListChannels does; streaming calls (terminal attach/type, file read/write streams) keep `require`, documented, since a stream can't be held and replayed (decision B2).
- Claims: `daemon/humatad/src/{identity,restart,terminal_session}.rs`, `daemon/humatad/tests/auth.rs` (worker sec15); `daemon/humatad/src/{inbox,inbox/deferred,inbox/kept,grant_target}.rs`, `daemon/humatad/tests/inbox.rs` (worker cards); `daemon/humatad/src/{session_service,channel_service,terminal,files}.rs`, `daemon/humatad/tests/{sessions,channels,files,terminal}.rs` (worker handlers); `daemon/humatad/tests/common/mod.rs` additive (sec15 only)
- Tasks:
  - [ ] B1 SEC-15 process-tree check · worker sec15 (Sonnet, medium) · done (uncompiled): Authenticate gains store + Processes; misuse revokes the tab token (replaced at next resume) and logs IDENTITY_MISUSED; `--admin` skips; auth.rs tests run `humata whoami` inside the tab; release-daemon token test rewritten to run inside the tab. Follow-up: cache holder pid in Identities (needs terminal.rs to call `set_holder_pid` after spawn) to drop the per-call store read
  - [ ] B2 previews + Always allow on deferred cards; store `deferred` flag · worker cards (Sonnet, medium) · done (uncompiled): AccessAheadPreview + Always allow when the call has an IN-22 target (GlobalRead has none); kept.rs reads `deferred`; entries saved before the flag restore as ordinary (stay pending)
  - [ ] B3 defer remaining unary handlers · worker handlers (Sonnet, medium) · done (uncompiled): `gate`/`answer_gated` in channel_service.rs; Channel Create/Get/Rename, Session Start/PreviewBranchNames/Get/Rename/Stop/Resume, Terminal ListTabs/OpenShellTab/OpenMachineShell/CloseTab/Type/PasteImage defer (approval re-runs the body with the gate skipped); Attach, ReadFile, WriteFile stay `require` (streams), SendQueuedNow (not built); `Terminals` → `{core: Arc<Core>}`
  - [ ] B4 reads allowed by default (D127.4): GlobalRead → Default Allow; tests set Ask explicitly; SPEC IN-17, BOUNDARIES, TESTING rows · worker reads (Sonnet, medium) · files `daemon/access/src/*`, `daemon/humatad/tests/{inbox,mcp,matrix}.rs`, `SPEC.md`, `api/BOUNDARIES.md`, `TESTING.md` · done (uncompiled): GlobalRead → DefaultAllow (access refusals.rs counts 13/16), tests set Ask explicitly, SPEC IN-17, BOUNDARIES, TESTING, MCP.md rows updated; global board writes need their own Ask capability when BOARD lands
- Seams (batch 2): S6 `api/proto/humata/store/v1/store.proto` `InboxEntry`: `bool deferred = 5;` (true for a request made by `Inbox::defer`); S7 serve.rs Authenticate gains `store.clone(), Arc::new(Kernel),` after `logins.clone()` in all four constructions (admin Anyone, admin Process(pid) before the `Admin` arg, both `new`); S8 serve.rs: move `let feed = …; let inbox = …;` above `let terminals = Terminals::new(…)` and add `inbox: inbox.clone(),` as the last `TerminalsConfig` field.
- Build gates (batch 2): gate 1 (main): S6–S8 applied, all build; 70/73 pass. Fixes (tests only, uncompiled): refusals.rs (stale under D127.4: default now allows cross-channel reads, Ask set explicitly); deferred.rs unit test (fixture never saved the session; inbox_requests FK); auth.rs (whoami tier is at `result.tier`; release-daemon test now runs whoami inside the tab after restart, since Attach has no credential on release). Rerun pending.
  Gate 2: 72/73; release-daemon auth_test lost its typed loop: `type_text` only queues input, and dropping the attach right after can close the stream before the line reaches the shell. Fix: the test writes whoami.sh, types `/bin/sh whoami.sh`, and waits for the script's `ready` file before dropping the attach. Rerun pending.
- Ready to commit (batch 2): after S6–S8 and a green gate. Tests: //apps/humata2/daemon/humatad:{humatad2_unit_test,auth_test,inbox_test,sessions_test,channels_test,terminal_test,mcp_test,matrix_test,access_test}, access crate tests (refusals), store tests; clippy humatad + access.
